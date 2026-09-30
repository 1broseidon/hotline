import { useState } from "react";
import { Refusal } from "../ui/Refusal";
import { wire, type McpOAuthStatus } from "../wire";

/**
 * Finishes an MCP server's OAuth sign-in from the address the browser landed
 * on (BRO-154).
 *
 * A desk on a server asks the provider to send the browser back to its own
 * loopback, which on the person's computer is nothing: the page fails to
 * load, and its address bar still holds the code and state the desk is
 * waiting for. Pasting it here hands them over on the desk's own wire. The
 * address is used once and never kept, shown again or written down.
 */
export function McpPasteBack({ loginId, onStatus }: { loginId: string; onStatus(status: McpOAuthStatus): void }) {
	const [address, setAddress] = useState("");
	const [busy, setBusy] = useState(false);
	/** What the desk said, if it refused; ours is the same sentence every time. */
	const [refused, setRefused] = useState<string | null>(null);

	const finish = async () => {
		setBusy(true);
		setRefused(null);
		try {
			const next = await wire.command("mcp.auth_callback", { loginId, callbackUrl: address.trim() });
			setAddress("");
			onStatus(next);
		} catch (error) {
			setRefused(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	return (
		<>
			<p className="group-hint">Approve in your browser, then paste the address of the page you land on (it won't load, and that's expected).</p>
			<label className="label mt-4" htmlFor="mcp-callback">
				Page address
			</label>
			<textarea
				id="mcp-callback"
				className="field h-20 w-full font-mono text-sm"
				placeholder="http://127.0.0.1:…"
				value={address}
				spellCheck={false}
				onChange={(event) => setAddress(event.target.value)}
			/>
			<div className="mt-3 flex justify-end">
				<button type="button" className="control btn-primary" disabled={busy || address.trim() === ""} onClick={() => void finish()}>
					{busy ? "Finishing…" : "Finish sign-in"}
				</button>
			</div>
			{refused !== null && <Refusal message="That address didn't finish the sign-in." detail={refused} />}
		</>
	);
}
