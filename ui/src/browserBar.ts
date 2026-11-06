import { invoke } from "@tauri-apps/api/core";

/**
 * Hotline's bar above a teammate's desk browser, in the pane the PIP opens
 * (crates/hotline-app/src/browser.rs). It says whose browser this is and
 * where it is, and offers Take control and Give it back, as the computer's
 * viewer does. The page below is a web view of its own that no capability
 * names; this bar reaches only the browser_bar_* commands. While the
 * teammate drives, the page is shielded from the person's clicks; while
 * the person has it, the teammate's acts wait and the bar can move the page.
 */

const persona = new URLSearchParams(location.hash.slice(1)).get("persona") ?? "";
const element = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const status = element("status");
const url = element<HTMLInputElement>("url");
const lock = element("lock");
const take = element<HTMLButtonElement>("take");
const error = element("error");
const untrusted = element("untrusted");
const untrustedLabel = element("untrusted-label");
const visit = element<HTMLButtonElement>("visit");
const steps = ["back", "forward", "reload"].map((id) => element<HTMLButtonElement>(id));

type BarState = {
	name: string;
	url: string;
	title: string;
	taken: boolean;
	/** "Certificate expired" when the last load was refused for its certificate. */
	untrusted: string | null;
	/** The page is on a certificate the person chose to visit anyway. */
	excepted: boolean;
};
let taken = false;
let editing = false;

let saying: ReturnType<typeof setTimeout> | undefined;
function say(message: string | null) {
	// A newer message keeps its own five seconds.
	clearTimeout(saying);
	error.hidden = !message;
	error.textContent = message ?? "";
	if (message) saying = setTimeout(() => say(null), 5000);
}

function draw(state: BarState) {
	taken = state.taken;
	document.body.classList.toggle("yours", taken);
	document.title = state.title ? `${state.name}: ${state.title}` : `${state.name}'s browser`;
	status.textContent = taken ? "You have control" : `${state.name} is driving`;
	take.textContent = taken ? "Give it back" : "Take control";
	url.readOnly = !taken;
	for (const step of steps) step.disabled = !taken;
	// Only the person, with control, visits past a refused certificate.
	untrusted.hidden = !state.untrusted;
	untrustedLabel.textContent = state.untrusted ?? "";
	visit.disabled = !taken;
	visit.title = taken ? "" : "Take control first";
	if (!editing) url.value = state.url;
	let host = "";
	try {
		const parsed = new URL(state.url);
		host = parsed.hostname;
		lock.className =
			host === "localhost" || /^(127\.|10\.|192\.168\.|\[::1\])/.test(host)
				? "local"
				: state.excepted
					? "excepted"
					: parsed.protocol === "https:"
					? "secure"
					: "";
	} catch {
		lock.className = "";
	}
}

async function refresh() {
	try {
		draw(await invoke<BarState>("browser_bar_state", { personaId: persona }));
	} catch (reason) {
		status.textContent = String(reason);
	}
}

take.addEventListener("click", async () => {
	try {
		await invoke("browser_bar_take", { personaId: persona, taken: !taken });
	} catch (reason) {
		say(String(reason));
	}
	await refresh();
});

for (const step of steps) {
	step.addEventListener("click", () =>
		invoke("browser_bar_go", { personaId: persona, step: step.id }).catch((reason) => say(String(reason))),
	);
}

visit.addEventListener("click", () => {
	visit.disabled = true;
	invoke("browser_bar_trust", { personaId: persona })
		.catch((reason) => say(String(reason)))
		.finally(refresh);
});

url.addEventListener("focus", () => {
	editing = !url.readOnly;
	if (editing) url.select();
});
url.addEventListener("blur", () => {
	editing = false;
});
element<HTMLFormElement>("where").addEventListener("submit", (event) => {
	event.preventDefault();
	if (url.readOnly) return;
	editing = false;
	url.blur();
	invoke("browser_bar_go", { personaId: persona, url: url.value }).catch((reason) => say(String(reason)));
});

refresh();
setInterval(refresh, 700);
