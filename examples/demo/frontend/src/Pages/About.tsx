import Layout from "../Layout";

import type { AboutProps } from "../types/AboutProps";

export default function About({ framework, adapter_version }: AboutProps) {
  return (
    <Layout title="About">
      <h1>About</h1>
      <p>
        Rendered by {framework} through loco-inertia v{adapter_version}.
      </p>
    </Layout>
  );
}
