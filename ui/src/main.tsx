import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { platform } from "./native";
import "./index.css";

document.documentElement.dataset.platform = platform();

const root = document.getElementById("root");
if (!root) throw new Error("The window has no root to draw into.");

createRoot(root).render(
	<StrictMode>
		<App />
	</StrictMode>,
);
