import { useForm } from "@inertiajs/react";
import { useState, type FormEvent } from "react";
import Layout from "../Layout";

const error = { color: "#b42318", fontSize: 14, margin: "4px 0 0" } as const;

type UploadForm = { name: string; avatar: File | null; photos: File[] };

export default function Upload() {
  // With a `File` in the data, Inertia submits multipart/form-data (`photos[]` per file).
  const form = useForm<UploadForm>({ name: "", avatar: null, photos: [] });
  // File inputs are not controlled by React: remount them to clear the chosen files.
  const [inputs, setInputs] = useState(0);

  const submit = (e: FormEvent) => {
    e.preventDefault();
    form.post("/upload", {
      onSuccess: () => {
        form.reset();
        setInputs((n) => n + 1);
      },
    });
  };

  return (
    <Layout title="Upload">
      <h1>Upload</h1>
      <form onSubmit={submit} style={{ display: "grid", gap: 12 }}>
        <label>
          Name
          <input
            style={{ display: "block", width: "100%", padding: 6, marginTop: 4 }}
            value={form.data.name}
            onChange={(e) => form.setData("name", e.target.value)}
            data-testid="name"
          />
          {form.errors.name && <p style={error}>{form.errors.name}</p>}
        </label>
        <label>
          Avatar (one image, max 1 MB)
          <input
            key={`avatar-${inputs}`}
            type="file"
            data-testid="avatar"
            onChange={(e) => form.setData("avatar", e.target.files?.[0] ?? null)}
          />
          {form.errors.avatar && <p style={error} data-testid="error-avatar">{form.errors.avatar}</p>}
        </label>
        <label>
          Photos (several images)
          <input
            key={`photos-${inputs}`}
            type="file"
            multiple
            data-testid="photos"
            onChange={(e) => form.setData("photos", Array.from(e.target.files ?? []))}
          />
          {form.errors.photos && <p style={error} data-testid="error-photos">{form.errors.photos}</p>}
        </label>
        {form.progress && <progress value={form.progress.percentage} max="100" />}
        <button type="submit" disabled={form.processing}>
          {form.processing ? "Uploading…" : "Upload"}
        </button>
      </form>
    </Layout>
  );
}
