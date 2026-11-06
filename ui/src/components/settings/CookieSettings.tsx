import { useEffect, useState } from "react";
import type { CookieHolder, CookieSources, Persona } from "../../generated/contract";

/** Who is on the team, as much as a row needs: its picture. */
type Member = { id: string; avatar?: Persona["avatar"] };
import { ChevronRightIcon } from "../../icons";
import { useCookiesRevision } from "../../room";
import { Avatar } from "../../ui/Avatar";
import { wire } from "../../wire";

function reason(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

/**
 * Settings › Secrets › Cookie Sources: each browser and profile on this
 * machine, a switch each. Off, the desk reads it for nothing, not even its
 * site names; the core holds that, this only says so. Listing them opens no
 * cookie store.
 */
export function CookieSourcesGroup() {
	const [sources, setSources] = useState<CookieSources | null>(null);
	const [busy, setBusy] = useState<string | null>(null);
	const [error, setError] = useState<string | null>(null);
	useEffect(() => {
		let gone = false;
		void wire
			.command("computer.cookies.sources", {})
			.then((found) => !gone && setSources(found))
			.catch((failed: unknown) => !gone && setError(reason(failed)));
		return () => {
			gone = true;
		};
	}, []);
	const set = async (browserId: string, profileId: string, on: boolean) => {
		setBusy(`${browserId}/${profileId}`);
		setError(null);
		try {
			setSources(await wire.command("computer.cookies.source.set", { browserId, profileId, on }));
		} catch (failed) {
			setError(reason(failed));
		} finally {
			setBusy(null);
		}
	};
	return (
		<section>
			<h3 className="group-title">Cookie Sources</h3>
			<div className="grouped">
				{sources === null ? (
					<p className="group-row text-sm text-ink-3">{error === null ? "Looking for browsers…" : "Couldn't look for browsers"}</p>
				) : sources.sources.length === 0 ? (
					<p className="group-row text-sm text-ink-3">No browser with cookies on this machine.</p>
				) : (
					sources.sources.map((source) => (
						<label key={`${source.browserId}/${source.profileId}`} className="group-row group-row-choice">
							<span className="group-row-text">
								<span className="group-row-title">
									{source.browserName} · {source.profileName}
								</span>
							</span>
							<input
								type="checkbox"
								className="switch"
								aria-label={`${source.browserName} · ${source.profileName}`}
								checked={source.on}
								disabled={busy !== null}
								onChange={(event) => void set(source.browserId, source.profileId, event.target.checked)}
							/>
						</label>
					))
				)}
			</div>
			<p className="group-hint">Hotline reads only these, and only to share cookies.</p>
			{sources?.blocked.map((line) => (
				<p key={line} className="group-hint">
					{line}
				</p>
			))}
			{error !== null && sources !== null && (
				<p role="alert" className="group-hint text-danger">
					{error}
				</p>
			)}
		</section>
	);
}

/**
 * Settings › Secrets › In Use: every teammate holding the person's cookies,
 * a row each with its sites, opening that teammate's pane. Absent while
 * nobody holds any.
 */
export function CookiesInUse({ team, onOpenTeammate }: { team: readonly Member[]; onOpenTeammate?: ((personaId: string) => void) | undefined }) {
	const [holders, setHolders] = useState<CookieHolder[]>([]);
	const revision = useCookiesRevision();
	// biome-ignore lint/correctness/useExhaustiveDependencies: revision is a count the room bumps to ask for a re-read.
	useEffect(() => {
		let gone = false;
		void wire
			.command("computer.cookies.holders", {})
			.then((found) => !gone && setHolders(found))
			.catch(() => !gone && setHolders([]));
		return () => {
			gone = true;
		};
	}, [revision]);
	if (holders.length === 0) return null;
	return (
		<section>
			<h3 className="group-title">In Use</h3>
			<div className="grouped">
				{holders.map((holder) => (
					<button
						key={holder.teammate}
						type="button"
						className="group-row group-row-choice w-full text-left"
						title={`Shared with ${holder.name}`}
						disabled={onOpenTeammate === undefined}
						onClick={() => onOpenTeammate?.(holder.teammate)}
					>
						<Avatar id={holder.teammate} name={holder.name} size={20} hash={team.find((persona) => persona.id === holder.teammate)?.avatar?.hash} />
						<span className="group-row-text">
							<span className="group-row-title">
								{holder.name}
								<span className="text-ink-3"> · {holder.sites.join(", ")}</span>
							</span>
						</span>
						<ChevronRightIcon className="shrink-0 text-ink-3" />
					</button>
				))}
			</div>
		</section>
	);
}
