import type { ComponentType } from "react";

const pages = import.meta.glob<{ default: ComponentType }>("./Pages/**/*.tsx", { eager: true });

export const title = (title: string) => (title ? `${title} · Loco + Inertia` : "Loco + Inertia");

export function resolve(name: string) {
  const page = pages[`./Pages/${name}.tsx`];
  if (!page) throw new Error(`Unknown Inertia page: ${name}`);
  return page;
}
