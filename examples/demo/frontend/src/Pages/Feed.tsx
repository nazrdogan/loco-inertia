import { Deferred, InfiniteScroll, router } from "@inertiajs/react";
import Layout from "../Layout";
import type { FeedProps } from "../types/FeedProps";

const card = { padding: "18px 12px", margin: "8px 0", border: "1px solid #8884", borderRadius: 8 } as const;

export default function Feed({ posts, even_only, stats, categories }: FeedProps) {
  // Toggling the filter starts the list over: `reset` drops the merged pages.
  const toggleEven = () =>
    router.visit("/feed", {
      data: even_only ? {} : { even: 1 },
      only: ["posts", "even_only"],
      reset: ["posts"],
      preserveScroll: false,
    });

  return (
    <Layout title="Feed">
      <h1>Feed</h1>

      <Deferred data="stats" fallback={<p>Loading stats…</p>}>
        <p data-testid="stats">
          {stats?.total_posts} posts in total (computed in {stats?.computed_in_ms} ms)
        </p>
      </Deferred>

      <h2>Categories</h2>
      {categories ? (
        <p data-testid="categories">{categories.join(", ")}</p>
      ) : (
        <button onClick={() => router.reload({ only: ["categories"] })}>Load categories</button>
      )}

      <h2>Posts</h2>
      <label>
        <input type="checkbox" checked={even_only} onChange={toggleEven} data-testid="even" /> Even ids only
      </label>
      <InfiniteScroll data="posts" buffer={200} loading={<p data-testid="loading">Loading more posts…</p>}>
        {posts.data.map((p) => (
          <article key={p.id} style={card} data-testid="post">
            {p.title}
          </article>
        ))}
      </InfiniteScroll>
    </Layout>
  );
}
