import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { platform } from "./native";
import "./index.css";

document.documentElement.dataset.platform = platform();

// A key pressed with a pointer does not keep the focus, the way a desktop
// button does not. Left to the web's default it would, and the next
// keystroke — Escape closing the pane it opened — promotes that quiet
// pointer focus to a keyboard ring on a key nobody is looking at. Fields
// keep theirs: a pointer in a field is where typing goes.
window.addEventListener("mousedown", (event) => {
	const target = event.target instanceof Element ? event.target : null;
	const key = target?.closest("button, [role='button']");
	if (key && !target?.closest("input, textarea, [contenteditable='true']")) event.preventDefault();
});

const root = document.getElementById("root");
if (!root) throw new Error("The window has no root to draw into.");

createRoot(root).render(
	<StrictMode>
		<App />
	</StrictMode>,
);
