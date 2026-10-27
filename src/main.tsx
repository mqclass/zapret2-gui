import React from "react";
import ReactDOM from "react-dom/client";
import { MotionConfig } from "framer-motion";
import App from "./App";
import "./index.css";
import { applyAccent, readStoredAccent } from "./utils/accent";

// Single theme: always "macos" (dark glass). No theme switching.
document.documentElement.dataset.theme = "macos";

// Apply the last saved accent synchronously, before the first render,
// so non-violet users never see a violet frame while settings load
// over IPC. The backend value confirms or overwrites this in App.
applyAccent(readStoredAccent());

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <MotionConfig reducedMotion="user">
      <App />
    </MotionConfig>
  </React.StrictMode>
);
