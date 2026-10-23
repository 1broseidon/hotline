import { useEffect, useState } from "react";
import type { AccountSetupView, SetupCopyReport } from "../../generated/contract";
import { isDesktop, openAccountSetup } from "../../native";
import { nameOf, onServer } from "../../serverFiles";
import { Refusal } from "../../ui/Refusal";
import { wire } from "../../wire";
import { SETUP_CATEGORIES, THIS_COMPUTER } from "../accounts/words";

const ROW_ORDER = ["instructions", "settings", "skills", "plugins", "mcp"];

function rank(id: string): number {
	const at = ROW_ORDER.indexOf(id);
	return at === -1 ? ROW_ORDER.length : at;
}

function plural(count: number, noun: string): string {
	return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

function copiedWords(report: SetupCopyReport): string {
	let words = `Copied ${plural(report.copied.length, "item")} from ${THIS_COMPUTER}.`;
	if (report.linksSkipped > 0) words += ` ${plural(report.linksSkipped, "link")} skipped.`;
	if (report.removedKeys.length > 0) words += ` Left out: ${report.removedKeys.join(", ")}.`;
	return words;
}

/** Open stands only on this computer's own desk: the shell opens a path on this disk, and only one the core listed. */
export function AccountSetup({ accountId, agentName }: { accountId: string; agentName: string }) {
	const [view, setView] = useState<AccountSetupView | null>(null);
	const [refusal, setRefusal] = useState<string | null>(null);
	const [said, setSaid] = useState<string | null>(null);
	const [choosing, setChoosing] = useState(false);
	const [ticked, setTicked] = useState<Set<string>>(new Set());
	const [busy, setBusy] = useState(false);
	const opens = isDesktop() && !onServer();

	const reload = () =>
		wire
			.command("accounts.setup.list", { accountId })
			.then(setView)
			.catch((error: Error) => {
				// Nothing to list, rather than reading forever; the refusal says why.
				setView((was) => was ?? { categories: [] });
				setRefusal(error.message);
			});

	// biome-ignore lint/correctness/useExhaustiveDependencies: read once per account; reload reads only the id and setters.
	useEffect(() => {
		void reload();
	}, [accountId]);

	const open = async (path: string) => {
		setRefusal(null);
		setSaid(null);
		try {
			await openAccountSetup(path);
			const name = nameOf(path);
			setSaid(/[/\\]$/.test(path) ? `The ${name} folder opened.` : `${name} opened in your editor.`);
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		}
	};

	const chosen = SETUP_CATEGORIES.filter((one) => ticked.has(one.id)).map((one) => one.id);
	const copy = async () => {
		if (busy || chosen.length === 0) return;
		setBusy(true);
		setRefusal(null);
		setSaid(null);
		try {
			const report = await wire.command("accounts.setup.copy", { accountId, categories: chosen });
			setChoosing(false);
			setSaid(copiedWords(report));
			await reload();
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	const flip = (category: string) =>
		setTicked((was) => {
			const next = new Set(was);
			if (next.has(category)) next.delete(category);
			else next.add(category);
			return next;
		});

	const origin = view?.setup;
	const rows = [...(view?.categories ?? [])].sort((a, b) => rank(a.id) - rank(b.id) || a.id.localeCompare(b.id));

	return (
		<>
			<section>
				<h3 className="group-title">Setup</h3>
				<div className="grouped">
					{view === null ? (
						<p className="group-row text-sm text-ink-3">Reading…</p>
					) : (
						origin !== undefined && (
							<p className="group-row text-sm text-ink-3">
								{origin.from === "empty"
									? "Started empty"
									: `Started from your ${agentName} setup on ${new Date(origin.at).toLocaleDateString(undefined, { day: "numeric", month: "short" })}`}
							</p>
						)
					)}
					{rows.map((row) => {
						const category = SETUP_CATEGORIES.find((one) => one.id === row.id);
						const item = category?.item ?? row.id;
						const detail = category?.noun !== undefined ? plural(row.count, category.noun) : "";
						return (
							<div key={row.id} className="group-row">
								<span className="group-row-text">
									<span className="group-row-title">{item}</span>
									{detail !== "" && <span className="group-row-detail">{detail}</span>}
								</span>
								{opens && row.path !== undefined && (
									<button type="button" className="control btn" aria-label={`Open ${item}`} onClick={() => void open(row.path!)}>
										Open
									</button>
								)}
							</div>
						);
					})}
				</div>
				<p className="group-hint">
					Opens in your own editor or file manager on this computer. Changes apply from the next session of the teammates on this account.
				</p>
			</section>
			{said !== null && (
				<p className="group-hint" role="status">
					{said}
				</p>
			)}
			<section>
				<h3 className="group-title">Copy from your own setup</h3>
				<div className="grouped">
					{!choosing ? (
						<div className="group-row">
							<span className="group-row-text">
								<span className="group-row-title">Copy again from {THIS_COMPUTER}</span>
								<span className="group-row-detail">Bring over what you changed in your own {agentName} setup since this account was made</span>
							</span>
							<button
								type="button"
								className="control btn"
								onClick={() => {
									// Settings stays as the account has it unless asked: copying it again undoes its own permissions and hooks.
									setTicked(new Set(SETUP_CATEGORIES.filter((one) => one.id !== "settings").map((one) => one.id)));
									setChoosing(true);
								}}
							>
								Choose…
							</button>
						</div>
					) : (
						<>
							{SETUP_CATEGORIES.map((category) => (
								<label key={category.id} className="group-row group-row-choice">
									<input type="checkbox" className="check" checked={ticked.has(category.id)} disabled={busy} onChange={() => flip(category.id)} />
									<span className="group-row-text">
										<span className="group-row-title">{category.choice}</span>
									</span>
								</label>
							))}
							<div className="group-row agent-account-caution flex-col items-stretch gap-2">
								<p className="text-sm text-ink-2">The ticked items replace this account's, including your changes to them. The sign-in is not touched.</p>
								<div className="flex justify-end gap-2">
									<button type="button" className="control btn-quiet" disabled={busy} onClick={() => setChoosing(false)}>
										Cancel
									</button>
									<button type="button" className="control btn btn-primary" disabled={busy || chosen.length === 0} onClick={() => void copy()}>
										Copy {plural(chosen.length, "item")}
									</button>
								</div>
							</div>
						</>
					)}
				</div>
				<p className="group-hint">The sign-in, keys and history are never copied.</p>
			</section>
			{refusal !== null && <Refusal message={refusal} />}
		</>
	);
}
