import React from "react";
import { createRoot } from "react-dom/client";
import { ProcessWindow } from "./ProcessWindow.jsx";
import { themeFor, themeTokens } from "../themes.js";
import "../styles.css";

for (const [name, value] of Object.entries(themeTokens(themeFor("midnight")))) document.documentElement.style.setProperty(name, value);
createRoot(document.getElementById("root")).render(<ProcessWindow />);
