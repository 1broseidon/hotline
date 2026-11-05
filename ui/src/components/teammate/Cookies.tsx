import { useEffect, useMemo, useState } from "react";
import type { CookieLend, CookieOfferSites, TranscriptEvent } from "../../generated/contract";
import { ChevronDownIcon, ChevronRightIcon, PlusIcon } from "../../icons";
import { useCookiesRevision } from "../../room";
import { Chips } from "../../ui/Chips";
import { wire } from "../../wire";
import { Favicon } from "../Favicon";
import { NESTED, RowText } from "./rows";

function reason(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

/** What a share's end says on its row: today, tomorrow, or the date. */
export function endsWords(endsAt: number, now = Date.now()): string {
	const day = (ms: number) => new Date(ms).toDateString();
	if (day(endsAt) === day(now)) return "Ends today";
	if (day(endsAt) === day(now + 24 * 60 * 60 * 1000)) return "Ends tomorrow";
	return `Ends ${new Date(endsAt).toLocaleDateString(undefined, { month: "short", day: "numeric" })}`;
}

/**
 * The person's cookies shared with this teammate's browser, its desk
 * browser's or its computer's alike: Add Cookies at the head, then a row a
 * site. Self-contained, so it can sit wherever the pane puts it. The rows
 * are read when the pane opens; the person's browsers are read only when
 * Add Cookies opens. A card the teammate raised and the person answered
 * with Use lands here too, so the list follows the tape's cards as well as
 * the room's records.
 */
export function Cookies({ personaId, disabled }: { personaId: string; disabled: boolean }) {
	const [lends, setLends] = useState<CookieLend[]>([]);
	const [error, setError] = useState<string | null>(null);
	const [adding, setAdding] = useState(false);
	const [open, setOpen] = useState<string | null>(null);
	const revision = useCookiesRevision();
	// biome-ignore lint/correctness/useExhaustiveDependencies: revision is a count the room bumps to ask for a re-read.
	useEffect(() => {
		let gone = false;
		let asked = 0;
		const refresh = () => {
			const request = ++asked;
			void wire
				.command("computer.cookies.lends", { personaId })
				.then((list) => {
					if (gone || request !== asked) return;
					setLends(Array.isArray(list) ? list : []);
					setError(null);
				})
				.catch((failed: unknown) => {
					if (!gone && request === asked) setError(reason(failed));
				});
		};
		refresh();
		const unsubscribe = wire.subscribe<TranscriptEvent>(
			{ tape: personaId },
			{
				snapshot: refresh,
				event: (event) => {
					if (event.kind === "cookie_offer") refresh();
				},
			},
		);
		return () => {
			gone = true;
			unsubscribe();
		};
	}, [personaId, revision]);

	const replace = (lend: CookieLend) => setLends((was) => was.map((one) => (one.id === lend.id ? lend : one)));

	return (
		<div>
			<h4 className="label mt-4">Cookies</h4>
			<div className="grouped">
				<button type="button" className="group-row group-row-add" aria-expanded={adding} disabled={disabled} onClick={() => setAdding((was) => !was)}>
					<PlusIcon />
					Add Cookies
				</button>
				{adding && (
					<AddCookies
						personaId={personaId}
						shared={lends.map((lend) => lend.site)}
						disabled={disabled}
						onShared={(lend) => setLends((was) => [...was.filter((one) => one.id !== lend.id), lend])}
					/>
				)}
				{lends.map((lend) => (
					<SharedRow
						key={lend.id}
						lend={lend}
						open={open === lend.id}
						disabled={disabled}
						onToggle={() => setOpen((was) => (was === lend.id ? null : lend.id))}
						onChange={replace}
						onRemoved={() => setLends((was) => was.filter((one) => one.id !== lend.id))}
					/>
				))}
			</div>
			{error !== null && (
				<p role="alert" className="group-hint text-danger">
					{error}
				</p>
			)}
		</div>
	);
}

/**
 * Add Cookies, open: a search field and the sites the person is signed
 * into across the sources switched on, each with where it is from. Reading
 * them is the desk's only look into the person's browsers before a press,
 * and it reads site names alone. A press shares the site.
 */
function AddCookies({
	personaId,
	shared,
	disabled,
	onShared,
}: {
	personaId: string;
	shared: string[];
	disabled: boolean;
	onShared(lend: CookieLend): void;
}) {
	const [found, setFound] = useState<CookieOfferSites | null>(null);
	const [query, setQuery] = useState("");
	const [busy, setBusy] = useState<string | null>(null);
	const [error, setError] = useState<string | null>(null);
	useEffect(() => {
		let gone = false;
		void wire
			.command("computer.cookies.lendable", {})
			.then((sites) => !gone && setFound(sites))
			.catch((failed: unknown) => !gone && setError(reason(failed)));
		return () => {
			gone = true;
		};
	}, []);

	const needle = query.trim().toLowerCase();
	const shown = useMemo(() => (found?.sites ?? []).filter((row) => row.site.includes(needle)), [found, needle]);

	const share = async (site: string) => {
		if (busy !== null || disabled) return;
		setBusy(site);
		setError(null);
		try {
			onShared(await wire.command("computer.cookies.lend", { personaId, site }));
		} catch (failed) {
			setError(reason(failed));
		} finally {
			setBusy(null);
		}
	};

	return (
		<>
			<div className={NESTED}>
				<input
					className="field flex-1 text-sm"
					aria-label="Search sites"
					placeholder="Search sites"
					autoComplete="off"
					spellCheck={false}
					value={query}
					onChange={(event) => setQuery(event.target.value)}
				/>
			</div>
			{found === null && error === null && (
				<p className={`${NESTED} text-sm text-ink-3`}>Reading your browsers…</p>
			)}
			{found !== null && shown.length === 0 && (
				<p className={`${NESTED} text-sm text-ink-3`}>{found.sites.length === 0 ? "No signed-in sites found." : "No matching sites."}</p>
			)}
			<div className="max-h-72 overflow-y-auto">
				{shown.map((row) => {
					const already = shared.includes(row.site);
					const from = [...new Set(row.sources.map((source) => source.browserName))].join(", ");
					return (
						<button
							key={row.site}
							type="button"
							className={`${NESTED} group-row-choice w-full text-left`}
							disabled={disabled || already || busy !== null}
							onClick={() => void share(row.site)}
						>
							<Favicon site={row.site} size={16} />
							<span className="group-row-text">
								<span className="group-row-title">{row.site}</span>
								<span className="group-row-detail">{busy === row.site ? "Sharing…" : from}</span>
							</span>
							{already && <span className="text-sm text-ink-3">Shared</span>}
						</button>
					);
				})}
			</div>
			{found?.unavailable.map((line) => (
				<p key={line} className={`${NESTED} group-row-detail`} style={{ whiteSpace: "normal" }}>
					{line}
				</p>
			))}
			{error !== null && (
				<p role="alert" className={`${NESTED} text-sm text-danger`} style={{ whiteSpace: "normal" }}>
					{error}
				</p>
			)}
		</>
	);
}

/**
 * One site's cookies shared with the teammate: the site's icon, the site,
 * and where they came from, with a chevron. The title names a choice only
 * when it is not the default. Opened, set in from the row: Keep fresh, Ends,
 * and Remove at the foot.
 */
function SharedRow({
	lend,
	open,
	disabled,
	onToggle,
	onChange,
	onRemoved,
}: {
	lend: CookieLend;
	open: boolean;
	disabled: boolean;
	onToggle(): void;
	onChange(lend: CookieLend): void;
	onRemoved(): void;
}) {
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const run = async (work: () => Promise<void>) => {
		if (busy || disabled) return;
		setBusy(true);
		setError(null);
		try {
			await work();
		} catch (failed) {
			setError(reason(failed));
		} finally {
			setBusy(false);
		}
	};
	const set = (change: { keepFresh?: boolean; ends?: "day" | "never" }) =>
		run(async () => onChange(await wire.command("computer.cookies.lend.set", { personaId: lend.teammate, id: lend.id, ...change })));
	const remove = () =>
		run(async () => {
			await wire.command("computer.cookies.remove", { personaId: lend.teammate, id: lend.id });
			onRemoved();
		});
	const choices = [lend.keepFresh === true ? "Keep fresh" : null, lend.endsAt !== undefined ? endsWords(lend.endsAt) : null].filter((one) => one !== null);
	const chevron = open ? <ChevronDownIcon className="shrink-0 text-ink-3" /> : <ChevronRightIcon className="shrink-0 text-ink-3" />;
	return (
		<>
			<button type="button" className="group-row group-row-choice w-full text-left" aria-expanded={open} onClick={onToggle}>
				<Favicon site={lend.site} size={16} />
				<span className="group-row-text">
					<span className="group-row-title">
						{lend.site}
						{choices.length > 0 && <span className="text-ink-3"> · {choices.join(" · ")}</span>}
					</span>
					<span className={`group-row-detail${error !== null ? " text-danger" : ""}`}>
						{error ?? (lend.waiting === true ? `From ${lend.browserName} · Not copied yet` : `From ${lend.browserName}`)}
					</span>
				</span>
				{chevron}
			</button>
			{open && (
				<>
					{lend.fromLaptop !== true && (
						<label className={`${NESTED} group-row-choice`}>
							<RowText title="Keep fresh" />
							<input
								type="checkbox"
								className="switch"
								checked={lend.keepFresh === true}
								disabled={busy || disabled}
								onChange={(event) => void set({ keepFresh: event.target.checked })}
							/>
						</label>
					)}
					<div className={NESTED}>
						<RowText title="Ends" />
						<Chips
							label={`When ${lend.site} ends`}
							value={lend.endsAt !== undefined ? "day" : "never"}
							choices={[
								{ id: "day", name: "1 day" },
								{ id: "never", name: "Never" },
							]}
							disabled={busy || disabled}
							onChange={(ends) => void set({ ends })}
						/>
					</div>
					<div className={NESTED}>
						<span className="group-row-text">
							<span className="group-row-detail">Shared {new Date(lend.lentAt).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" })}</span>
						</span>
						<button type="button" className="control btn-quiet btn-sm text-danger" title="Stop sharing" disabled={busy || disabled} onClick={() => void remove()}>
							Remove
						</button>
					</div>
				</>
			)}
		</>
	);
}
