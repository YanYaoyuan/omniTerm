import React from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App.jsx";
import "@xterm/xterm/css/xterm.css";
import "./styles.css";

// Each terminal owns a real process; avoid StrictMode's development remount.
createRoot(document.getElementById("root")).render(<App />);
