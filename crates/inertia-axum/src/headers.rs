//! Header names used by the Inertia protocol.

/// Marks requests from the Inertia client and its JSON responses.
pub const X_INERTIA: &str = "x-inertia";
/// The asset version the client was loaded with.
pub const X_INERTIA_VERSION: &str = "x-inertia-version";
/// With a `409`: the URL the client should load as a full page visit.
pub const X_INERTIA_LOCATION: &str = "x-inertia-location";
/// Partial reload: the component the client is reloading.
pub const X_INERTIA_PARTIAL_COMPONENT: &str = "x-inertia-partial-component";
/// Partial reload: the prop paths to send (comma separated).
pub const X_INERTIA_PARTIAL_DATA: &str = "x-inertia-partial-data";
/// Partial reload: the prop paths to leave out (comma separated).
pub const X_INERTIA_PARTIAL_EXCEPT: &str = "x-inertia-partial-except";
/// Merge props the client drops instead of merging (comma separated).
pub const X_INERTIA_RESET: &str = "x-inertia-reset";
/// The error bag validation errors of this request go under.
pub const X_INERTIA_ERROR_BAG: &str = "x-inertia-error-bag";
/// Infinite scroll: `append` or `prepend` the loaded page.
pub const X_INERTIA_INFINITE_SCROLL_MERGE_INTENT: &str = "x-inertia-infinite-scroll-merge-intent";
/// Once props the client already has (comma separated keys).
pub const X_INERTIA_EXCEPT_ONCE_PROPS: &str = "x-inertia-except-once-props";
/// Marks a Precognition (validate only) request and its response.
pub const PRECOGNITION: &str = "precognition";
/// Precognition: the fields to validate (comma separated).
pub const PRECOGNITION_VALIDATE_ONLY: &str = "precognition-validate-only";
/// Precognition: set on a response without errors.
pub const PRECOGNITION_SUCCESS: &str = "precognition-success";
