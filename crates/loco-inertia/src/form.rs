//! [`InertiaForm`]: validated form input, like Laravel's form requests.

use std::cell::RefCell;

use axum::{
    body::Bytes,
    extract::{FromRequest, FromRequestParts, Multipart, Request},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{
    de::{
        self, value::MapDeserializer, value::SeqDeserializer, DeserializeOwned, IntoDeserializer,
    },
    forward_to_deserialize_any, Deserialize, Deserializer,
};
use serde_json::{Map, Value};
use validator::Validate;

use crate::{validation_errors, Inertia};

/// A submitted form that has passed validation.
///
/// Accepts what the Inertia client sends: JSON (`useForm` without files), or
/// `multipart/form-data` (forms with files, or `forceFormData`) and
/// `application/x-www-form-urlencoded`. Form fields use Inertia's bracket names
/// (`user[name]`, `tags[0]`) and are converted to the field types of `T` (`"42"` → `u32`,
/// `"1"`/`"on"` → `true`, `""` → `None`); files become [`UploadedFile`]s.
///
/// - On a Precognition request (`form.validate(...)` in the client) it answers `204` or
///   `422` itself and the handler never runs, so live validation cannot trigger the action.
/// - When validation fails it redirects back with the errors (under the request's error bag,
///   if any), so they show up in `form.errors`.
/// - Otherwise the handler gets the valid data.
///
/// ```ignore
/// async fn submit(inertia: Inertia, InertiaForm(form): InertiaForm<ContactForm>) -> Result<Response> {
///     save(&form).await?;
///     Ok(inertia.redirect("/contact").with_flash("message", "Sent!").into_response())
/// }
/// ```
///
/// Arrays of objects need indexed names (`items[0][name]`): with Inertia's default
/// `name[]` format each field of such an object lands in its own element, as in PHP. Set
/// `queryStringArrayFormat: 'indices'` on those forms. Request bodies are subject to the body
/// size limit (Loco's `limit_payload`).
#[derive(Debug, Clone)]
pub struct InertiaForm<T>(pub T);

impl<T, S> FromRequest<S> for InertiaForm<T>
where
    T: DeserializeOwned + Validate,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let (mut parts, body) = req.into_parts();
        let inertia = Inertia::from_request_parts(&mut parts, state)
            .await
            .map_err(IntoResponse::into_response)?;
        let content_type = parts
            .headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let req = Request::from_parts(parts, body);

        let form: T = if content_type.starts_with("multipart/form-data") {
            let multipart = Multipart::from_request(req, state)
                .await
                .map_err(IntoResponse::into_response)?;
            let (fields, files) = read_multipart(multipart).await?;
            from_form_fields(fields, files)?
        } else if content_type.starts_with("application/x-www-form-urlencoded") {
            let body = Bytes::from_request(req, state)
                .await
                .map_err(IntoResponse::into_response)?;
            let fields: Vec<_> = form_urlencoded::parse(&body)
                .into_owned()
                .map(|(name, value)| (name, FieldValue::Text(value)))
                .take(MAX_FIELDS + 1)
                .collect();
            if fields.len() > MAX_FIELDS {
                return Err(too_many_fields());
            }
            from_form_fields(fields, Vec::new())?
        } else {
            let Json(form) = Json::<T>::from_request(req, state)
                .await
                .map_err(IntoResponse::into_response)?;
            form
        };

        let errors = form.validate().err().map(|e| validation_errors(&e));
        if inertia.is_precognitive() {
            return Err(inertia.precognition_response(errors.unwrap_or_default()));
        }
        match errors {
            Some(errors) => Err(inertia.back().with_errors(errors).into_response()),
            None => Ok(Self(form)),
        }
    }
}

/// A file from a multipart form. Use it as a field type (`avatar: Option<UploadedFile>`,
/// `photos: Vec<UploadedFile>`) of the struct an [`InertiaForm`] deserializes.
#[derive(Clone, PartialEq, Eq)]
pub struct UploadedFile {
    /// The name the browser sent, if any. Untrusted: do not use it as a path.
    pub file_name: Option<String>,
    pub content_type: Option<String>,
    pub bytes: Bytes,
}

impl std::fmt::Debug for UploadedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UploadedFile")
            .field("file_name", &self.file_name)
            .field("content_type", &self.content_type)
            .field("len", &self.bytes.len())
            .finish()
    }
}

/// Serializes the metadata only (`file_name`, `content_type`, `size`), never the bytes, so a
/// file can appear in validation error params or logs without leaking its contents.
impl serde::Serialize for UploadedFile {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("UploadedFile", 3)?;
        s.serialize_field("file_name", &self.file_name)?;
        s.serialize_field("content_type", &self.content_type)?;
        s.serialize_field("size", &self.bytes.len())?;
        s.end()
    }
}

/// Marks where a file sits in the field tree while deserializing. Only files put it there:
/// text fields stay strings whatever they contain.
const FILE_MARKER: &str = "$inertia_file";

/// Deepest bracket nesting accepted in a field name (`a[b][c]` is 3). Building and
/// deserializing the field tree recurses once per level, so unbounded names could overflow
/// the stack and abort the process.
const MAX_FIELD_DEPTH: usize = 32;

/// Most fields (text and files) accepted in one form, like PHP's `max_input_vars`. Bounds the
/// work a single request can cause beyond what the body size limit allows.
const MAX_FIELDS: usize = 1000;

fn too_many_fields() -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        format!("Invalid form data: a form may have at most {MAX_FIELDS} fields"),
    )
        .into_response()
}

/// A form field's value.
enum FieldValue {
    Text(String),
    /// Index into the uploaded files.
    File(usize),
}

thread_local! {
    /// The files of the form being deserialized (deserialization is synchronous, so the
    /// thread-local is set and cleared around it).
    static FILES: RefCell<Vec<UploadedFile>> = const { RefCell::new(Vec::new()) };
}

impl<'de> Deserialize<'de> for UploadedFile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct FileVisitor;

        impl<'de> de::Visitor<'de> for FileVisitor {
            type Value = UploadedFile;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an uploaded file (multipart/form-data)")
            }

            fn visit_map<A: de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<UploadedFile, A::Error> {
                let mut index = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key == FILE_MARKER {
                        index = Some(map.next_value::<usize>()?);
                    } else {
                        map.next_value::<de::IgnoredAny>()?;
                    }
                }
                let index = index.ok_or_else(|| de::Error::custom("expected an uploaded file"))?;
                FILES
                    .with(|files| files.borrow().get(index).cloned())
                    .ok_or_else(|| de::Error::custom("uploaded file is missing"))
            }
        }

        deserializer.deserialize_map(FileVisitor)
    }
}

// The error is the rejection response itself, returned to the client as is.
#[allow(clippy::result_large_err)]
async fn read_multipart(
    mut multipart: Multipart,
) -> Result<(Vec<(String, FieldValue)>, Vec<UploadedFile>), Response> {
    let bad = |e: axum::extract::multipart::MultipartError| e.into_response();
    let mut fields = Vec::new();
    let mut files = Vec::new();
    while let Some(field) = multipart.next_field().await.map_err(bad)? {
        if fields.len() == MAX_FIELDS {
            return Err(too_many_fields());
        }
        let Some(name) = field.name().map(ToString::to_string) else {
            continue;
        };
        match field.file_name().map(ToString::to_string) {
            Some(file_name) => {
                let content_type = field.content_type().map(ToString::to_string);
                let bytes = field.bytes().await.map_err(bad)?;
                // An empty file input: no file chosen.
                if file_name.is_empty() && bytes.is_empty() {
                    fields.push((name, FieldValue::Text(String::new())));
                    continue;
                }
                fields.push((name, FieldValue::File(files.len())));
                files.push(UploadedFile {
                    file_name: Some(file_name),
                    content_type,
                    bytes,
                });
            }
            None => fields.push((name, FieldValue::Text(field.text().await.map_err(bad)?))),
        }
    }
    Ok((fields, files))
}

/// Deserialize bracket-named form fields (and files) into `T`.
// The error is the rejection response itself, returned to the client as is.
#[allow(clippy::result_large_err)]
fn from_form_fields<T: DeserializeOwned>(
    fields: Vec<(String, FieldValue)>,
    files: Vec<UploadedFile>,
) -> Result<T, Response> {
    let tree = fields_to_tree(fields).map_err(|e| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid form data: {e}"),
        )
            .into_response()
    })?;
    FILES.with(|f| *f.borrow_mut() = files);
    let result = T::deserialize(Lenient(tree));
    FILES.with(|f| f.borrow_mut().clear());
    result.map_err(|e| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid form data: {e}"),
        )
            .into_response()
    })
}

/// `user[address][street]=x`, `tags[0]=a`, `tags[]=b` → nested objects and arrays.
fn fields_to_tree(fields: Vec<(String, FieldValue)>) -> Result<Value, String> {
    let mut root = Map::new();
    for (name, value) in fields {
        let value = match value {
            FieldValue::Text(text) => Value::String(text),
            FieldValue::File(index) => serde_json::json!({ FILE_MARKER: index }),
        };
        let path = split_brackets(&name);
        if path.len() > MAX_FIELD_DEPTH {
            return Err(format!(
                "field names may nest at most {MAX_FIELD_DEPTH} levels deep"
            ));
        }
        // Clients cannot name a field after the file marker to pass text off as a file.
        if path.iter().any(|segment| segment == FILE_MARKER) {
            continue;
        }
        insert_path(&mut root, &path, value);
    }
    Ok(arrays_from_numeric_objects(Value::Object(root)))
}

fn split_brackets(name: &str) -> Vec<String> {
    let (head, rest) = name.split_once('[').map_or((name, ""), |(h, r)| (h, r));
    let mut path = vec![head.to_string()];
    for part in rest.split('[') {
        if let Some(key) = part.strip_suffix(']') {
            path.push(key.to_string());
        }
    }
    path
}

fn insert_path(map: &mut Map<String, Value>, path: &[String], value: Value) {
    let (key, rest) = match path.split_first() {
        Some(split) => split,
        None => return,
    };
    // `tags[]`: the next free index, after any explicit ones (`tags[1]=a&tags[]=b`).
    let key = if key.is_empty() {
        map.keys()
            .filter_map(|k| k.parse::<usize>().ok())
            .max()
            .map_or(map.len(), |max| max + 1)
            .to_string()
    } else {
        key.clone()
    };
    if rest.is_empty() {
        map.insert(key, value);
        return;
    }
    let child = map.entry(key).or_insert_with(|| Value::Object(Map::new()));
    if !child.is_object() {
        *child = Value::Object(Map::new());
    }
    if let Value::Object(child) = child {
        insert_path(child, rest, value);
    }
}

/// Objects whose keys are all indices (`{"0": a, "1": b}`) become arrays, in index order.
fn arrays_from_numeric_objects(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let is_list = !map.is_empty() && map.keys().all(|k| k.parse::<usize>().is_ok());
            if is_list {
                let mut items: Vec<(usize, Value)> = map
                    .into_iter()
                    .map(|(k, v)| (k.parse().unwrap_or(0), arrays_from_numeric_objects(v)))
                    .collect();
                items.sort_by_key(|(i, _)| *i);
                Value::Array(items.into_iter().map(|(_, v)| v).collect())
            } else {
                Value::Object(
                    map.into_iter()
                        .map(|(k, v)| (k, arrays_from_numeric_objects(v)))
                        .collect(),
                )
            }
        }
        other => other,
    }
}

/// Deserializes form values (all strings) by the target type, like `serde_urlencoded` does:
/// `"42"` into numbers, `"1"`/`"true"`/`"on"` into `true`, `""` into `None`.
struct Lenient(Value);

type Error = serde_json::Error;

impl<'de> IntoDeserializer<'de, Error> for Lenient {
    type Deserializer = Self;

    fn into_deserializer(self) -> Self {
        self
    }
}

fn parse<T: std::str::FromStr>(s: &str, what: &str) -> Result<T, Error> {
    s.trim()
        .parse()
        .map_err(|_| de::Error::custom(format!("invalid {what}: {s:?}")))
}

macro_rules! lenient_number {
    ($($method:ident => $parse:ty, $visit:ident;)*) => {$(
        fn $method<V: de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
            match self.0 {
                Value::String(s) => visitor.$visit(parse::<$parse>(&s, "number")?),
                other => other.$method(visitor),
            }
        }
    )*};
}

impl<'de> Deserializer<'de> for Lenient {
    type Error = Error;

    fn deserialize_any<V: de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::Array(items) => {
                let mut seq = SeqDeserializer::new(items.into_iter().map(Lenient));
                let value = visitor.visit_seq(&mut seq)?;
                seq.end()?;
                Ok(value)
            }
            Value::Object(map) => {
                let mut map = MapDeserializer::new(map.into_iter().map(|(k, v)| (k, Lenient(v))));
                let value = visitor.visit_map(&mut map)?;
                map.end()?;
                Ok(value)
            }
            other => other.deserialize_any(visitor),
        }
    }

    fn deserialize_bool<V: de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
                "1" | "true" | "on" | "yes" => visitor.visit_bool(true),
                "0" | "false" | "off" | "no" | "" => visitor.visit_bool(false),
                _ => Err(de::Error::custom(format!("invalid boolean: {s:?}"))),
            },
            Value::Null => visitor.visit_bool(false),
            other => other.deserialize_bool(visitor),
        }
    }

    lenient_number! {
        deserialize_i8 => i64, visit_i64;
        deserialize_i16 => i64, visit_i64;
        deserialize_i32 => i64, visit_i64;
        deserialize_i64 => i64, visit_i64;
        deserialize_u8 => u64, visit_u64;
        deserialize_u16 => u64, visit_u64;
        deserialize_u32 => u64, visit_u64;
        deserialize_u64 => u64, visit_u64;
        deserialize_f32 => f64, visit_f64;
        deserialize_f64 => f64, visit_f64;
    }

    fn deserialize_option<V: de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match &self.0 {
            Value::Null => visitor.visit_none(),
            Value::String(s) if s.is_empty() => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_unit<V: de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match &self.0 {
            Value::Null => visitor.visit_unit(),
            Value::String(s) if s.is_empty() => visitor.visit_unit(),
            _ => self.deserialize_any(visitor),
        }
    }

    fn deserialize_string<V: de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::Number(n) => visitor.visit_string(n.to_string()),
            Value::Bool(b) => visitor.visit_string(b.to_string()),
            other => Lenient(other).deserialize_any(visitor),
        }
    }

    fn deserialize_str<V: de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_string(visitor)
    }

    fn deserialize_seq<V: de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            // A single value where a list is expected (one checkbox of a group).
            Value::String(s) if s.is_empty() => {
                Lenient(Value::Array(Vec::new())).deserialize_any(visitor)
            }
            Value::String(s) => {
                Lenient(Value::Array(vec![Value::String(s)])).deserialize_any(visitor)
            }
            other => Lenient(other).deserialize_any(visitor),
        }
    }

    fn deserialize_newtype_struct<V: de::Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: de::Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.0.deserialize_enum(name, variants, visitor)
    }

    forward_to_deserialize_any! {
        i128 u128 char bytes byte_buf unit_struct tuple tuple_struct map struct identifier
        ignored_any
    }
}
