import { useCallback, useEffect, useState } from "react";
import { levelWord } from "../../asides";
import type { InboundKind, InboundSource } from "../../generated/contract";
import { Refusal, refused, type Refused } from "../../ui/Refusal";
import { wire } from "../../wire";

const UNREAD = "Couldn't read sources";
const UNSAVED = "Couldn't change source";

const KIND_WORDS: Record<InboundKind, string> = {
	webhook: "Webhook",
	mail: "Mailbox",
	github: "GitHub",
	other: "Other",
};

/** A teammate as this section names one. */
type Teammate = { id: string; name: string };

/**
 * Settings → Inbound: the places outside Hotline the person lets leave
 * teammates asides (`docs/design/thread-awareness.md`, "Inbound sources").
 * Each row says what it is, whom it may aside, how loudly, how often and
 * whether it carries files, with its switch (off is muted) and Remove. There
 * is no add form: a source comes from a teammate's proposal card, which the
 * person answers, or from `inbound.add`; and no transport delivers through
 * one yet.
 */
export function InboundSection({ team }: { team: readonly Teammate[] }) {
	const [sources, setSources] = useState<InboundSource[] | null>(null);
	const [refusal, setRefusal] = useState<Refused | null>(null);
	const [removing, setRemoving] = useState<string | null>(null);
	const [busy, setBusy] = useState(false);

	const refresh = useCallback(() => {
		void wire
			.command("inbound.list", {})
			.then(setSources)
			.catch((error: unknown) => setRefusal(refused(UNREAD, error)));
	}, []);
	useEffect(() => {
		refresh();
	}, [refresh]);

	const apply = async (change: () => Promise<unknown>) => {
		setBusy(true);
		setRefusal(null);
		try {
			await change();
			setRemoving(null);
			refresh();
		} catch (error) {
			setRefusal(refused(UNSAVED, error));
		} finally {
			setBusy(false);
		}
	};

	return (
		<section>
			<h3 className="group-title">Inbound sources</h3>
			<div className="grouped">
				{sources === null ? (
					<p className="group-row text-sm text-ink-3">{refusal === null ? "…" : "Not available."}</p>
				) : sources.length === 0 ? (
					<p className="group-row text-sm text-ink-3">None yet.</p>
				) : (
					sources.map((source) => (
						<div key={source.id} className="group-row">
							<span className="group-row-text min-w-0">
								<span className="group-row-title">{source.name}</span>
								<span className="group-row-detail">{detail(source, team)}</span>
							</span>
							{removing === source.id ? (
								<span className="flex shrink-0 items-center gap-2">
									<button type="button" className="control btn-quiet btn-sm" onClick={() => setRemoving(null)}>
										Keep
									</button>
									<button
										type="button"
										className="control btn btn-sm text-danger"
										disabled={busy}
										onClick={() => void apply(() => wire.command("inbound.remove", { id: source.id }))}
									>
										Remove
									</button>
								</span>
							) : (
								<button type="button" className="control btn-quiet btn-sm" disabled={busy} onClick={() => setRemoving(source.id)}>
									Remove…
								</button>
							)}
							<input
								type="checkbox"
								role="switch"
								className="switch"
								aria-label={`Hear ${source.name}`}
								aria-checked={source.muted !== true}
								checked={source.muted !== true}
								disabled={busy}
								onChange={(event) => void apply(() => wire.command("inbound.update", { id: source.id, muted: !event.target.checked }))}
							/>
						</div>
					))
				)}
			</div>
			<p className="group-hint">
				Places outside Hotline that may leave teammates asides, heard as untrusted notes, never instructions. A teammate
				proposes one on a card for you to answer. Nothing delivers through one yet.
			</p>
			{refusal !== null && <Refusal {...refusal} />}
		</section>
	);
}

/** What a source may do, in a line: "GitHub · to Quill · note · 10 an hour". */
function detail(source: InboundSource, team: readonly Teammate[]): string {
	const names = source.teammates.map((id) => team.find((one) => one.id === id)?.name ?? "a teammate");
	return [
		KIND_WORDS[source.type],
		`to ${names.join(", ")}`,
		levelWord(source.maxLevel),
		`${source.rate} an hour`,
		source.filesAllowed === true ? "files" : undefined,
	]
		.filter((part) => part !== undefined)
		.join(" · ");
}
