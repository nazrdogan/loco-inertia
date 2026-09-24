import { useForm } from "@inertiajs/react";
import type { FormEvent } from "react";
import Layout from "../Layout";
import type { ContactForm } from "../types/ContactForm";

const field = { display: "block", width: "100%", padding: 6, marginTop: 4 } as const;
const error = { color: "#b42318", fontSize: 14, margin: "4px 0 0" } as const;

type Field = keyof ContactForm;

export default function Contact() {
  // Method + URL make the form precognitive: `validate(field)` asks the server to check
  // the data without submitting it.
  const form = useForm<ContactForm>("post", "/contact", { name: "", email: "", message: "" });

  const submit = (e: FormEvent) => {
    e.preventDefault();
    form.submit({ onSuccess: () => form.reset() });
  };

  const input = (name: Field, multiline = false) => {
    const props = {
      style: field,
      value: form.data[name],
      onChange: (e: { target: { value: string } }) => form.setData(name, e.target.value),
      onBlur: () => form.validate(name),
      "data-testid": `input-${name}`,
    };
    return multiline ? <textarea rows={4} {...props} /> : <input {...props} />;
  };

  const status = (name: Field) =>
    form.invalid(name) ? (
      <p style={error} data-testid={`error-${name}`}>
        {form.errors[name]}
      </p>
    ) : // `reset()` clears `touched` but not `valid`, so check both.
    form.touched(name) && form.valid(name) ? (
      <p style={{ ...error, color: "#067647" }} data-testid={`valid-${name}`}>
        ✓
      </p>
    ) : null;

  return (
    <Layout title="Contact">
      <h1>Contact</h1>
      <form onSubmit={submit} noValidate style={{ display: "grid", gap: 12 }}>
        <label>
          Name
          {input("name")}
          {status("name")}
        </label>
        <label>
          Email
          {input("email")}
          {status("email")}
        </label>
        <label>
          Message
          {input("message", true)}
          {status("message")}
        </label>
        <button type="submit" disabled={form.processing}>
          {form.processing ? "Sending…" : form.validating ? "Checking…" : "Send"}
        </button>
      </form>
    </Layout>
  );
}
