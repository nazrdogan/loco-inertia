import { createInertiaApp } from "@inertiajs/react";
import createServer from "@inertiajs/react/server";
import { renderToString } from "react-dom/server";
import { resolve, title } from "./pages";

// POST /render on port 13714 (INERTIA_SSR_PORT to change it); Loco calls it on first visits.
createServer(
  (page) =>
    createInertiaApp({
      page,
      render: renderToString,
      resolve,
      title,
      setup: ({ App, props }) => <App {...props} />,
    }),
  { port: Number(process.env.INERTIA_SSR_PORT ?? 13714), host: "127.0.0.1" },
);
