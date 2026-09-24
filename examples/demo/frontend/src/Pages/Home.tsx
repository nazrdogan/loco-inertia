import Layout from "../Layout";

import type { HomeProps } from "../types/HomeProps";

export default function Home({ greeting, features }: HomeProps) {
  return (
    <Layout title="Home">
      <h1>{greeting}</h1>
      <ul>
        {features.map((f) => (
          <li key={f}>{f}</li>
        ))}
      </ul>
    </Layout>
  );
}
