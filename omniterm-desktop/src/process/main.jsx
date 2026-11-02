import React from "react";
import { createRoot } from "react-dom/client";
import { ProcessWindow } from "./ProcessWindow.jsx";
import { themeFor, themeTokens } from "../themes.js";
import "../styles.css";

const query = new URLSearchParams(location.search);
document.documentElement.classList.add(`platform-${query.get("platform") || "linux"}`);
function applyTheme(id) {
  for (const [name, value] of Object.entries(themeTokens(themeFor(id)))) document.documentElement.style.setProperty(name, value);
}
applyTheme(query.get("theme") || "midnight");
window.omniterm.onWindowTheme(applyTheme);
createRoot(document.getElementById("root")).render(<ProcessWindow />);
