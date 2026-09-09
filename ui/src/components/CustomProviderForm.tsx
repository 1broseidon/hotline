import { useState } from "react";
import type { Credential, OpenAiApi } from "../generated/contract";
import { Refusal } from "../ui/Refusal";
import { wire } from "../wire";

export function CustomProviderForm({ credential, onSaved, onCancel }: {
	credential: Credential | undefined;
	onSaved(credential: Credential): void;
	onCancel(): void;
}) {
	const [name, setName] = useState(credential?.label ?? "");
	const [baseUrl, setBaseUrl] = useState(credential?.baseUrl ?? "");
	const [api, setApi] = useState<OpenAiApi>(credential?.custom?.api ?? "responses");
	const [useKey, setUseKey] = useState(credential?.credentialKind === "api_key");
	const [secret, setSecret] = useState("");
	const [models, setModels] = useState(credential?.custom?.models.join("\n") ?? "");
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);
	const [discovered, setDiscovered] = useState<number | null>(null);

	const keyInput = (): string | undefined => {
		if (!useKey) return "";
		if (secret.trim()) return secret.trim();
		if (credential?.credentialKind === "api_key") return undefined;
		throw new Error("Enter an API key or turn off API key authentication.");
	};

	const run = async (work: () => Promise<void>) => {
		if (busy) return;
		setBusy(true);
		setRefusal(null);
		try { await work(); }
		catch (error) { setRefusal(error instanceof Error ? error.message : String(error)); }
		finally { setBusy(false); }
	};

	const discover = () => run(async () => {
		const key = keyInput();
		const ids = await wire.command("credential.custom_models", {
			...(credential ? { id: credential.id } : {}), baseUrl, ...(key === undefined ? {} : { secret: key }),
		});
		setModels(ids.join("\n"));
		setDiscovered(ids.length);
	});

	const save = () => run(async () => {
		const key = keyInput();
		const saved = await wire.command("credential.custom_save", {
			...(credential ? { id: credential.id } : {}),
			draft: { name, baseUrl, api, models: models.split("\n"), ...(key === undefined ? {} : { secret: key }) },
		});
		onSaved(saved);
	});

	return (
		<form onSubmit={(event) => { event.preventDefault(); void save(); }}>
			<h3 className="group-title">{credential ? "Edit connection" : "OpenAI-compatible connection"}</h3>
			<fieldset disabled={busy} className="grouped min-w-0">
				<div className="group-row">
					<label className="w-24 shrink-0 text-sm text-ink-2" htmlFor="custom-name">Name</label>
					<input id="custom-name" className="field min-w-0 flex-1" value={name} onChange={(event) => setName(event.target.value)} placeholder="LM Studio or Together AI" required />
				</div>
				<div className="group-row">
					<label className="w-24 shrink-0 text-sm text-ink-2" htmlFor="custom-url">Base URL</label>
					<input id="custom-url" type="url" className="field min-w-0 flex-1 font-mono text-sm" value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} placeholder="http://localhost:1234/v1" spellCheck={false} autoComplete="off" required />
				</div>
				<div className="group-row">
					<label className="w-24 shrink-0 text-sm text-ink-2" htmlFor="custom-api">API</label>
					<select id="custom-api" className="field min-w-0 flex-1" value={api} onChange={(event) => setApi(event.target.value as OpenAiApi)}>
						<option value="responses">Responses</option>
						<option value="chat_completions">Chat Completions</option>
					</select>
				</div>
				<label className="group-row text-sm text-ink-2">
					<input type="checkbox" checked={useKey} onChange={(event) => { setUseKey(event.target.checked); setSecret(""); }} />
					Use API key authentication
				</label>
				{useKey && <div className="group-row flex-wrap">
					<label className="w-24 shrink-0 text-sm text-ink-2" htmlFor="custom-key">API key</label>
					<input id="custom-key" type="password" className="field min-w-0 flex-1 font-mono text-sm" value={secret} onChange={(event) => setSecret(event.target.value)} placeholder={credential?.credentialKind === "api_key" ? "Leave blank to keep the saved key" : "API key"} autoComplete="off" spellCheck={false} />
					{credential?.credentialKind === "api_key" && <p className="w-full text-sm text-ink-3">Enter the key again when changing the base URL.</p>}
				</div>}
				<div className="group-row flex-col items-stretch gap-2">
					<div className="flex items-center justify-between gap-2">
						<label className="text-sm text-ink-2" htmlFor="custom-models">Model IDs</label>
						<button type="button" className="control btn-quiet" disabled={!baseUrl.trim()} onClick={() => void discover()}>Discover models</button>
					</div>
					<textarea id="custom-models" className="field min-h-32 w-full font-mono text-sm" rows={5} value={models} onChange={(event) => setModels(event.target.value)} placeholder="One model ID per line" spellCheck={false} required />
					<p className="text-sm text-ink-3">Discover models or enter IDs manually. Discovery replaces this list; review it and add or remove IDs before saving. Choose models that support tool calls.</p>
					{discovered !== null && <p className="text-sm text-ink-3" role="status">{discovered === 0 ? "No models were returned. Enter a model ID manually." : `Found ${discovered} models.`}</p>}
				</div>
				<div className="group-row justify-end">
					<button type="button" className="control btn-quiet" onClick={onCancel}>Cancel</button>
					<button type="submit" className="control btn-primary" disabled={!name.trim() || !baseUrl.trim() || !models.trim()}>{busy ? "Working…" : "Save connection"}</button>
				</div>
			</fieldset>
			{refusal !== null && <Refusal message={refusal} />}
		</form>
	);
}
