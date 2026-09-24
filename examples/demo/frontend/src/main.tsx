import { createInertiaApp } from "@inertiajs/react";
import "./app.css";
import { resolve, title } from "./pages";

// Without `setup`, Inertia hydrates server-rendered markup (`data-server-rendered`) and
// renders from scratch otherwise.
createInertiaApp({ resolve, title });
