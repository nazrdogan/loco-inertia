{% set file_name = name | snake_case -%}
{% set module_name = file_name | pascal_case -%}
{% set page_name = action.name | pascal_case -%}
to: frontend/src/Pages/{{module_name}}/{{page_name}}.tsx
skip_exists: true
message: "Page `{{module_name}}/{{page_name}}` was added successfully."
---
import type { {{module_name}}{{page_name}}Props } from "../../types/{{module_name}}{{page_name}}Props";

export default function {{module_name}}{{page_name}}({ title }: {{module_name}}{{page_name}}Props) {
  return (
    <main>
      <h1>{title}</h1>
      <p>
        Edit <code>frontend/src/Pages/{{module_name}}/{{page_name}}.tsx</code> and{" "}
        <code>src/controllers/{{file_name}}.rs</code>.
      </p>
    </main>
  );
}
