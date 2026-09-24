import { Head, Link, usePage } from "@inertiajs/react";
import type { ReactNode } from "react";

export default function Layout({ title, children }: { title: string; children: ReactNode }) {
  // Shared props and flash data are typed from Rust via src/inertia.d.ts.
  const page = usePage();
  const message = page.flash.message;

  return (
    <main style={{ fontFamily: "system-ui, sans-serif", maxWidth: 640, margin: "3rem auto", padding: "0 16px" }}>
      <Head title={title} />
      <nav style={{ display: "flex", gap: 16, marginBottom: 24, alignItems: "baseline" }}>
        <strong data-testid="app-name">{page.props.app_name}</strong>
        <Link href="/">Home</Link>
        <Link href="/about">About</Link>
        <Link href="/feed">Feed</Link>
        <Link href="/contact">Contact</Link>
        <Link href="/upload">Upload</Link>
      </nav>
      {message && (
        <p role="status" data-testid="flash" style={{ background: "#ecfdf3", color: "#067647", padding: "8px 12px", borderRadius: 6 }}>
          {message}
        </p>
      )}
      {children}
      <footer style={{ marginTop: 32, fontSize: 13, opacity: 0.7 }} data-testid="loaded-at">
        Shared data loaded at {page.props.loaded_at}
      </footer>
    </main>
  );
}
