import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { DeskRoot } from "./App";
import { platform } from "./native";
import { startAccent } from "./accent";
import { startTheme } from "./theme";
import { quietTooltipsOnPress } from "./tooltips";
import { keepFocusOffPressedKeys } from "./pressFocus";
import "./index.css";

document.documentElement.dataset["platform"] = platform();
// WebKitGTK leaves a tooltip up after a click (tooltips.ts).
if (platform() === "linux") quietTooltipsOnPress();
startTheme();
startAccent();
// A pressed key keeps no focus (pressFocus.ts).
keepFocusOffPressedKeys();

const root = document.getElementById("root");
if (!root) throw new Error("The window has no root to draw into.");

createRoot(root).render(
	<StrictMode>
		<DeskRoot />
	</StrictMode>,
);
