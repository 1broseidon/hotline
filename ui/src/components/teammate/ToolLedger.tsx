import { useEffect, useState } from "react";
import type { SessionState, TeammateToolLedger, ToolLedgerRow } from "../../generated/contract";
import { CheckIcon, WarningIcon } from "../../icons";
import type { McpServer } from "../../mcp";
import { wire } from "../../wire";
import { FoldRow, NESTED, RowText } from "./rows";

/**
 * What this teammate actually had at its last start: the grants above are
 * the intent, this is the outcome. One row carries the count, and the
 * tools that failed to attach are always shown under it, because those
 * are the only rows that need a person; the rest open on press. A ledger
 * is a fact of a start, not a live feed, so it is read when the pane
 * opens and again when the session changes state, the only time a new
 * one can exist.
 */
export function ToolLedger({
	personaId,
	sessionState,
	servers,
}: {
	personaId: string;
	sessionState: SessionState;
	servers: McpServer[];
}) {
	const [ledger, setLedger] = useState<TeammateToolLedger | null | undefined>(undefined);
	const [open, setOpen] = useState(false);

	useEffect(() => {
		setLedger(undefined);
		setOpen(false);
	}, [personaId]);

	// The answer replaces what is shown; it never blanks it first.
	useEffect(() => {
		let cancelled = false;
		void wire
			.command("teammate.tools", { personaId })
			.then((next) => {
				if (!cancelled) setLedger(next);
			})
			.catch(() => {
				if (!cancelled) setLedger(null);
			});
		return () => {
			cancelled = true;
		};
	}, [personaId, sessionState]);

	if (ledger === undefined) return null;

	const rows = ledger?.rows ?? [];
	const missing = rows.filter((row) => row.state === "absent");
	const foldable = rows.length > missing.length;
	const shown = open ? rows : missing;
	const value =
		ledger === null ? "none yet" : `${rows.length}${missing.length > 0 ? `, ${missing.length} missing` : ""}`;

	return (
		<>
			{foldable ? (
				<FoldRow title="Tools" value={value} open={open} onToggle={() => setOpen((was) => !was)} />
			) : (
				<div className="group-row">
					<RowText title="Tools" value={value} />
				</div>
			)}
			{groupedByOrigin(shown).map(([origin, items]) => (
				<div key={origin}>
					<p className="border-y border-line bg-hover py-1 pr-3 pl-7 text-xs text-ink-3" title={origin}>
						{originName(origin, servers)}
					</p>
					{items.map((row) => (
						<div key={`${row.source}-${row.origin}-${row.name}`} className={`${NESTED} items-start gap-2 py-2`}>
							<span className="mt-px shrink-0" title={row.state}>
								{row.state === "verified" ? (
									<CheckIcon className="text-accent-ink" />
								) : row.state === "absent" ? (
									<WarningIcon className="text-danger" />
								) : (
									<span className="grid h-4 w-4 place-items-center">
										<span className="h-1.5 w-1.5 rounded-full" style={{ boxShadow: "inset 0 0 0 1.5px var(--ink-4)" }} />
									</span>
								)}
							</span>
							<span className="group-row-text">
								<span className="group-row-title font-mono text-sm">{row.name}</span>
								<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
									{row.reason}
								</span>
							</span>
						</div>
					))}
				</div>
			))}
		</>
	);
}

/** The supplier as a person knows it: Hotline Agent, or the server's name from Settings → Tools. */
function originName(origin: string, servers: McpServer[]): string {
	if (origin === "hotline") return "Hotline Agent";
	return servers.find((server) => server.id === origin)?.name ?? origin;
}

function groupedByOrigin(rows: ToolLedgerRow[]): [string, ToolLedgerRow[]][] {
	const groups = new Map<string, ToolLedgerRow[]>();
	for (const row of rows) {
		const known = groups.get(row.origin);
		if (known) known.push(row);
		else groups.set(row.origin, [row]);
	}
	return [...groups.entries()].map(([origin, items]) => [
		origin,
		items.slice().sort((a, b) => a.name.localeCompare(b.name)),
	]);
}
