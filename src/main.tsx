import React from "react";
import ReactDOM from "react-dom/client";
// Prologue type system: Lora (wordmark/headings), IBM Plex Sans (UI),
// IBM Plex Mono (anything git) — bundled, no runtime font CDN.
import "@fontsource/lora/400.css";
import "@fontsource/lora/600.css";
import "@fontsource/ibm-plex-sans/400.css";
import "@fontsource/ibm-plex-sans/400-italic.css";
import "@fontsource/ibm-plex-sans/500.css";
import "@fontsource/ibm-plex-sans/600.css";
import "@fontsource/ibm-plex-sans/700.css";
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/500.css";
import "@fontsource/ibm-plex-mono/600.css";
import "@fontsource/ibm-plex-mono/700.css";
import App from "./App";
import { isTauri } from "./platform";
// Amp look for the browser UI, scoped to :root[data-ui="web"] so the Mac
// app keeps Prologue's brand.
import "./web-theme.css";

if (!isTauri()) {
  document.documentElement.dataset.ui = "web";
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
