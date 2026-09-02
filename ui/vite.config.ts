import react from "@vitejs/plugin-react";
import tailwind from "@tailwindcss/vite";
import { defineConfig } from "vite";

// The shell opens the window on this port in development and refuses to guess
// another one, so a port already in use is a mistake to hear about rather than
// a second instance to start quietly beside the first.
export default defineConfig({
	plugins: [react(), tailwind()],
	server: { port: 5174, strictPort: true },
	build: { target: "es2022" },
});
