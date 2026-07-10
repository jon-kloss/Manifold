import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

// Bundled locally per the license posture — no network fonts at runtime.
import "@fontsource/rajdhani/500.css";
import "@fontsource/rajdhani/600.css";
import "@fontsource/rajdhani/700.css";
import "@fontsource/barlow/400.css";
import "@fontsource/barlow/500.css";
import "@fontsource/barlow/600.css";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/500.css";
import "@fontsource/jetbrains-mono/700.css";
import "./tokens/tokens.css";
import "./base.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
