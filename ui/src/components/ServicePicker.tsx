import { useState } from "react";
import type { Provider } from "../generated/contract";
import { CheckIcon } from "../icons";
import { ProviderRow, connectionMethod } from "./ConnectProvider";

/**
 * The services most people already pay for, by the name the desk lists
 * them under, with the name people call them by. They are cards; the rest
 * of the desk's list waits behind More, so the choice fits without scrolling.
 */
const POPULAR: { name: string; title: string; by?: string }[] = [
	{ name: "Anthropic", title: "Anthropic", by: "Claude" },
	{ name: "ChatGPT", title: "ChatGPT", by: "Your plan" },
	{ name: "OpenRouter", title: "OpenRouter", by: "Every model" },
	{ name: "xAI", title: "xAI", by: "Grok" },
	{ name: "Ollama Local", title: "Ollama", by: "On this computer" },
	{ name: "GitHub Copilot", title: "Copilot", by: "GitHub" },
];

/**
 * Choosing a service to connect, the same in the welcome and in Settings ›
 * Providers: six cards, then "N more services" opening the rest as a list.
 * A service already connected shows as done on its card and leaves the list.
 */
export function ServicePicker({
	providers,
	connected,
	disabled = false,
	onPick,
}: {
	providers: Provider[];
	/** Names of the services already connected. */
	connected: readonly string[];
	disabled?: boolean;
	onPick(provider: Provider): void;
}) {
	const [more, setMore] = useState(false);
	const cards = POPULAR.flatMap((one) => {
		const provider = providers.find((candidate) => candidate.name === one.name);
		return provider === undefined ? [] : [{ provider, title: one.title, by: one.by }];
	});
	const rest = providers.filter((provider) => !POPULAR.some((one) => one.name === provider.name));
	return (
		<>
			<div className="welcome-services">
				{cards.map(({ provider, title, by }) => {
					const done = connected.includes(provider.name);
					return (
						<button
							key={provider.id}
							type="button"
							className="welcome-service"
							data-done={done ? "" : undefined}
							disabled={disabled || done}
							onClick={() => onPick(provider)}
						>
							<span className="welcome-service-title">
								{title}
								{done && <CheckIcon className="text-accent" />}
							</span>
							<span className="welcome-service-text">
								{done ? "Connected" : [by, provider.credentialKinds.map(connectionMethod).join(" or ")].filter(Boolean).join(" · ")}
							</span>
						</button>
					);
				})}
			</div>
			{rest.length > 0 && (
				<button type="button" className="control btn-quiet self-center" aria-expanded={more} onClick={() => setMore((was) => !was)}>
					{more ? "Fewer services" : `${rest.length} more services, like Gemini and Mistral`}
				</button>
			)}
			{more && (
				<div className="grouped welcome-list">
					{rest
						.filter((provider) => !connected.includes(provider.name))
						.map((provider) => (
							<ProviderRow key={provider.id} provider={provider} disabled={disabled} onPick={() => onPick(provider)} />
						))}
				</div>
			)}
		</>
	);
}
