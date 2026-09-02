import { useEffect, useState } from "react";
import { wire, type Credential } from "../wire";
import { Sheet } from "./Sheet";

/** The providers Toad can run a model on today, in the order they are offered. */
const PROVIDERS = [
	{ id: "anthropic", name: "Anthropic" },
	{ id: "openai", name: "OpenAI" },
	{ id: "openrouter", name: "OpenRouter" },
] as const;

/**
 * Provider keys.
 *
 * The secret goes in and never comes back out: the vault holds it and the room
 * remembers only that a key exists, so this list can say which providers are
 * signed in without ever being able to show one.
 */
export function Keys({ onClose }: { onClose(): void }) {
	const [held, setHeld] = useState<Credential[]>([]);
	const [providerId, setProviderId] = useState<string>(PROVIDERS[0].id);
	const [secret, setSecret] = useState("");
	const [busy, setBusy] = useState(false);
	const [refusal, setRefusal] = useState<string | null>(null);

	useEffect(() => {
		wire
			.command("credential.list", {})
			.then(setHeld)
			.catch((error: Error) => setRefusal(error.message));
	}, []);

	const submit = async () => {
		if (!secret.trim() || busy) return;
		setBusy(true);
		setRefusal(null);
		try {
			const label = PROVIDERS.find((one) => one.id === providerId)?.name ?? providerId;
			const made = await wire.command("credential.create", {
				providerId,
				label,
				secret: secret.trim(),
			});
			setHeld((known) => [...known, made]);
			setSecret("");
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	return (
		<Sheet title="Keys" onClose={onClose}>
			{held.length > 0 && (
				<ul className="mb-4 flex flex-col gap-1">
					{held.map((one) => (
						<li
							key={one.id}
							className="flex items-center gap-2 rounded-lg bg-paper-3 px-2.5 py-1.5 text-xs"
						>
							<span className="font-medium text-ink-2">{one.label}</span>
							<span className="font-mono text-ink-3">{one.providerId}</span>
							<span className="ml-auto text-ink-3">{one.revoked ? "revoked" : "in use"}</span>
						</li>
					))}
				</ul>
			)}

			<form
				className="flex flex-col gap-3"
				onSubmit={(event) => {
					event.preventDefault();
					void submit();
				}}
			>
				<div>
					<label className="label" htmlFor="key-provider">
						Provider
					</label>
					<select
						id="key-provider"
						className="field"
						value={providerId}
						onChange={(event) => setProviderId(event.target.value)}
					>
						{PROVIDERS.map((one) => (
							<option key={one.id} value={one.id}>
								{one.name}
							</option>
						))}
					</select>
				</div>

				<div>
					<label className="label" htmlFor="key-secret">
						API key
					</label>
					<input
						id="key-secret"
						type="password"
						className="field font-mono text-xs"
						autoFocus
						spellCheck={false}
						value={secret}
						onChange={(event) => setSecret(event.target.value)}
					/>
				</div>

				{refusal !== null && <p className="text-xs text-[var(--danger)]">{refusal}</p>}

				<div className="mt-1 flex justify-end gap-2">
					<button type="button" className="btn-quiet" onClick={onClose}>
						Done
					</button>
					<button type="submit" className="btn-primary" disabled={busy || secret.trim() === ""}>
						Save key
					</button>
				</div>
			</form>
		</Sheet>
	);
}
