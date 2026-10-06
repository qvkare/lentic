import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

// Self-hosted faces (web/DESIGN.md, Type): bundled by Vite, no network at runtime.
import "@fontsource/barlow/300.css";
import "@fontsource/barlow/400.css";
import "@fontsource/barlow/500.css";
import "@fontsource/barlow-semi-condensed/300.css";
import "@fontsource/barlow-semi-condensed/400.css";
import "@fontsource/source-serif-4/400-italic.css";
import "@fontsource/jetbrains-mono/400.css";

import { App } from "./App";
import "./styles.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
