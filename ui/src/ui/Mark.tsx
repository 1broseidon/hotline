import claude from "../assets/marks/claude.svg";
import copilot from "../assets/marks/copilot.svg";
import cursor from "../assets/marks/cursor.svg";
import deepseek from "../assets/marks/deepseek.svg";
import google from "../assets/marks/google.svg";
import kimi from "../assets/marks/kimi.svg";
import meta from "../assets/marks/meta.svg";
import mistral from "../assets/marks/mistral.svg";
import ollama from "../assets/marks/ollama.svg";
import openai from "../assets/marks/openai.svg";
import openrouter from "../assets/marks/openrouter.svg";
import qwen from "../assets/marks/qwen.svg";
import xai from "../assets/marks/xai.svg";
import xiaomi from "../assets/marks/xiaomi.svg";

/**
 * Who serves a model, who made it, and which agent runs a teammate, as each
 * draws itself (assets/marks, from thesvg). A mark in its maker's colours is
 * an image; a one-colour mark is a mask filled with the ink, so it follows
 * light and dark with the window. Nothing here is put into the page as
 * markup: the files are only ever pictures.
 */
const MARKS: Record<string, { url: string; mono: boolean }> = {
	anthropic: { url: claude, mono: false },
	openai: { url: openai, mono: true },
	xai: { url: xai, mono: true },
	google: { url: google, mono: false },
	ollama: { url: ollama, mono: true },
	openrouter: { url: openrouter, mono: true },
	mistral: { url: mistral, mono: false },
	deepseek: { url: deepseek, mono: false },
	qwen: { url: qwen, mono: true },
	meta: { url: meta, mono: false },
	xiaomi: { url: xiaomi, mono: false },
	kimi: { url: kimi, mono: false },
	copilot: { url: copilot, mono: true },
	cursor: { url: cursor, mono: true },
};

/** A model's maker, read off its id and name: the family names each maker gives its models. */
const MAKERS: [RegExp, string][] = [
	[/claude|anthropic/, "anthropic"],
	[/gpt|openai|\bo[34]\b|codex/, "openai"],
	[/grok|x-ai|xai/, "xai"],
	[/gemini|gemma|google/, "google"],
	[/mistral|mixtral|devstral|codestral|magistral/, "mistral"],
	[/deepseek/, "deepseek"],
	[/qwen/, "qwen"],
	[/llama|meta/, "meta"],
	[/mimo|xiaomi/, "xiaomi"],
	[/kimi|moonshot/, "kimi"],
];

/** The maker of a model, else whoever serves it (`provider/model`). */
export function modelMark(modelId: string, name = ""): string | null {
	const [provider = "", ...rest] = modelId.toLowerCase().split("/");
	const said = `${rest.join("/")} ${name.toLowerCase()}`;
	for (const [family, mark] of MAKERS) if (family.test(said)) return mark;
	return providerMark(provider);
}

/** The provider a picker's section heading names ("Anthropic", "Ollama Local", "OpenRouter"). */
export function providerMark(name: string): string | null {
	const said = name.toLowerCase().replace(/[^a-z]/g, "");
	if (said.startsWith("anthropic") || said.startsWith("claude")) return "anthropic";
	if (said.startsWith("openai") || said.startsWith("chatgpt")) return "openai";
	if (said.startsWith("xai") || said.startsWith("grok")) return "xai";
	if (said.startsWith("google") || said.startsWith("gemini")) return "google";
	if (said.startsWith("githubcopilot") || said.startsWith("copilot")) return "copilot";
	for (const id of ["ollama", "openrouter", "mistral", "deepseek", "qwen", "kimi", "xiaomi", "cursor"]) if (said.startsWith(id)) return id;
	return null;
}

/** The agent a teammate runs on, by its backend's id. */
export function harnessMark(backendId: string): string | null {
	const said = backendId.toLowerCase();
	if (said.startsWith("claude")) return "anthropic";
	if (said.startsWith("codex")) return "openai";
	if (said.startsWith("gemini") || said.startsWith("antigravity")) return "google";
	if (said.startsWith("cursor")) return "cursor";
	if (said.startsWith("grok")) return "xai";
	return null;
}

/**
 * Whether an agent serves other labs' models as its own (Cursor runs
 * Claude, GPT, Gemini and Grok): its mark stands for whatever it runs, not
 * the model's maker's.
 */
export function hostsModels(backendId: string): boolean {
	return backendId.toLowerCase().startsWith("cursor");
}

export function Mark({ id, size = 14 }: { id: string | null; size?: number }) {
	const mark = id === null ? undefined : MARKS[id];
	if (mark === undefined) return <span aria-hidden="true" className="mark mark-none" style={{ width: size, height: size }} />;
	return mark.mono ? (
		<span aria-hidden="true" className="mark mark-mono" style={{ width: size, height: size, maskImage: `url("${mark.url}")`, WebkitMaskImage: `url("${mark.url}")` }} />
	) : (
		<img aria-hidden="true" alt="" className="mark" src={mark.url} width={size} height={size} />
	);
}
