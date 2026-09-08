import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { tauriTransport } from "./ipc/tauriTransport";
import { setTransport } from "./ipc/transport";

// Composition root: the only place the real transport is installed.
setTransport(tauriTransport);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
