import { useCallback, useEffect, useState } from "react";
import type { SkillEntry } from "../generated/contract";
import { ChevronRightIcon, PlusIcon } from "../icons";
import { pickDirectory, revealPath } from "../native";
import { useRoomSettings } from "../room";
import { BackKey, Band } from "../ui/Band";
import { Refusal } from "../ui/Refusal";
import { Scroll } from "../ui/Scroll";
import { wire } from "../wire";

/** Where the person's own skills are read from when the room does not say. */
const STANDARD_FOLDER = "~/.agents/skills";

/** One word for where a skill came from, as the row's second line says it. */
export function skillSourceName(entry: SkillEntry): string {
	switch (entry.source) {
		case "builtin":
			return "Built in";
		case "gateway":
			return "Gateway";
		case "home":
			return "Your folder";
		case "workspace":
			return "Workspace";
		case "computer":
			return "Computer";
	}
}

/**
 * Skills: the person's own folder, the gateway and the built-ins, drawn the
 * way Tools is. The person's own skills live where other agents read them
 * too, `~/.agents/skills` unless changed here; the switch on a row offers
 * that skill to teammates, and nothing is copied. The gateway takes a folder
 * picked from anywhere and copies it in. A row opens the skill's page, where
 * a gateway skill is removed at the foot. An invalid entry stays in the list
 * with its reason, so the person can fix the folder rather than wonder
 * where it went. Which teammate may read a skill is a different question,
 * answered on that teammate.
 */
export function SkillsSection({ onBack }: { onBack?: (() => void) | undefined }) {
	const [entries, setEntries] = useState<SkillEntry[]>([]);
	const [refusal, setRefusal] = useState<string | null>(null);
	const [open, setOpen] = useState<string | null>(null);
	const [busy, setBusy] = useState(false);
	const { skillsHome } = useRoomSettings();

	const refresh = useCallback(() => {
		void wire
			.command("skills.list", {})
			.then(setEntries)
			.catch((error: Error) => setRefusal(error.message));
	}, []);

	// The folder may change under the list, so a new folder lists again.
	useEffect(() => {
		refresh();
	}, [refresh, skillsHome]);

	const run = async (work: () => Promise<void>) => {
		setBusy(true);
		setRefusal(null);
		try {
			await work();
			refresh();
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	const add = async () => {
		const path = await pickDirectory();
		if (path === null) return;
		await run(async () => {
			await wire.command("skills.add", { path });
		});
	};
	const remove = (name: string) =>
		run(async () => {
			await wire.command("skills.remove", { name });
			setOpen(null);
		});
	const offer = (name: string, offered: boolean) =>
		run(async () => {
			await wire.command("skills.offer", { name, offered });
		});
	const changeFolder = async () => {
		const path = await pickDirectory();
		if (path === null) return;
		await run(async () => {
			await wire.command("settings.update", { patch: { skillsHome: path } });
		});
	};
	const standardFolder = () =>
		run(async () => {
			await wire.command("settings.update", { patch: { skillsHome: null } });
		});

	const opened = entries.find((one) => `${one.source}:${one.name}` === open);
	if (opened !== undefined) {
		return (
			<SkillPage
				entry={opened}
				busy={busy}
				refusal={refusal}
				onBack={() => setOpen(null)}
				onRemove={() => void remove(opened.name)}
			/>
		);
	}

	const home = entries.filter((one) => one.source === "home");
	const builtins = entries.filter((one) => one.source === "builtin");
	const gateway = entries.filter((one) => one.source === "gateway");

	return (
		<div className="pane">
			<Band>
				{onBack !== undefined && <BackKey onBack={onBack} />}
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Skills</h2>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					<section>
						<h3 className="group-title">Your skills</h3>
						<div className="grouped">
							<div className="group-row">
								<span className="group-row-text">
									<span className="group-row-title">Folder</span>
									<span className="group-row-detail selectable font-mono" style={{ whiteSpace: "normal", wordBreak: "break-all" }}>
										{skillsHome ?? STANDARD_FOLDER}
									</span>
								</span>
								{skillsHome !== null && (
									<button type="button" className="control btn-quiet" disabled={busy} onClick={() => void standardFolder()}>
										Standard
									</button>
								)}
								<button type="button" className="control btn-quiet" disabled={busy} onClick={() => void changeFolder()}>
									Change
								</button>
							</div>
							{home.length === 0 ? (
								<p className="group-row text-sm text-ink-3">Nothing there yet.</p>
							) : (
								home.map((entry) => (
									<HomeRow
										key={entry.name}
										entry={entry}
										busy={busy}
										onOpen={() => setOpen(`${entry.source}:${entry.name}`)}
										onOffer={(offered) => void offer(entry.name, offered)}
									/>
								))
							)}
						</div>
						<p className="group-hint">
							Your own skills, in the folder other agents read too. Switch one on to offer it to teammates: each is granted it in
							its pane and reads it fresh at every start. Nothing is copied here, and an entry that is not a skill says why.
						</p>
					</section>
					<section>
						<h3 className="group-title">Gateway</h3>
						<div className="grouped">
							<button type="button" className="group-row group-row-add" disabled={busy} onClick={() => void add()}>
								<PlusIcon />
								Add a skill folder
							</button>
							{gateway.length === 0 ? (
								<p className="group-row text-sm text-ink-3">No skills yet.</p>
							) : (
								gateway.map((entry) => <SkillRow key={entry.name} entry={entry} onOpen={() => setOpen(`${entry.source}:${entry.name}`)} />)
							)}
						</div>
						<p className="group-hint">
							A skill from anywhere else: a folder holding SKILL.md with a name and a description, copied in. Every skill here is
							offered; grant skills per teammate, in its pane.
						</p>
					</section>
					<section>
						<h3 className="group-title">Built in</h3>
						<div className="grouped">
							{builtins.map((entry) => (
								<SkillRow key={entry.name} entry={entry} onOpen={() => setOpen(`${entry.source}:${entry.name}`)} />
							))}
						</div>
						<p className="group-hint">Every teammate has these.</p>
					</section>
					{refusal !== null && <Refusal message={refusal} />}
				</div>
			</Scroll>
		</div>
	);
}

function SkillRow({ entry, onOpen }: { entry: SkillEntry; onOpen(): void }) {
	return (
		<button type="button" className="group-row group-row-choice w-full text-left" onClick={onOpen}>
			<span className="group-row-text">
				<span className="group-row-title">{entry.name}</span>
				<span className={entry.invalid !== undefined ? "group-row-detail text-danger" : "group-row-detail"}>
					{entry.invalid ?? entry.description}
				</span>
			</span>
			<ChevronRightIcon className="shrink-0 text-ink-3" />
		</button>
	);
}

/** One of the person's own: the row opens its page, the switch offers it. An entry that is not a skill has no switch, only its reason. */
function HomeRow({
	entry,
	busy,
	onOpen,
	onOffer,
}: {
	entry: SkillEntry;
	busy: boolean;
	onOpen(): void;
	onOffer(offered: boolean): void;
}) {
	return (
		<div className="group-row group-row-choice">
			<button type="button" className="group-row-text min-w-0 flex-1 text-left" onClick={onOpen}>
				<span className="group-row-title">{entry.name}</span>
				<span className={entry.invalid !== undefined ? "group-row-detail text-danger" : "group-row-detail"}>
					{entry.invalid ?? entry.description}
				</span>
			</button>
			{entry.invalid === undefined ? (
				<input
					type="checkbox"
					className="switch"
					aria-label={`Offer ${entry.name} to teammates`}
					checked={entry.offered === true}
					disabled={busy}
					onChange={(event) => onOffer(event.target.checked)}
				/>
			) : (
				<ChevronRightIcon className="shrink-0 text-ink-3" />
			)}
		</div>
	);
}

/** One skill's page: what it is for, where it is, and for a gateway skill the way out at the foot. */
function SkillPage({
	entry,
	busy,
	refusal,
	onBack,
	onRemove,
}: {
	entry: SkillEntry;
	busy: boolean;
	refusal: string | null;
	onBack(): void;
	onRemove(): void;
}) {
	const onDisk = entry.source === "gateway" || entry.source === "home";
	return (
		<div className="pane">
			<Band>
				<BackKey onBack={onBack} />
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">{entry.name}</h2>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					<section>
						<div className="grouped">
							<div className="group-row">
								<span className="group-row-text">
									<span className="group-row-title">
										{skillSourceName(entry)}
										{entry.version !== undefined && <span className="text-ink-3"> · release {entry.version}</span>}
										{entry.source === "home" && (
											<span className="text-ink-3"> · {entry.offered === true ? "offered to teammates" : "not offered"}</span>
										)}
									</span>
									{entry.invalid !== undefined ? (
										<span className="group-row-detail text-danger">{entry.invalid}</span>
									) : (
										<span className="group-row-detail selectable" style={{ whiteSpace: "normal" }}>
											{entry.description}
										</span>
									)}
								</span>
							</div>
							<div className="group-row">
								<span className="group-row-text">
									<span className="group-row-title">{entry.source === "builtin" ? "In every workspace at" : "Folder"}</span>
									<span className="group-row-detail selectable font-mono" style={{ whiteSpace: "normal", wordBreak: "break-all" }}>
										{entry.path}
									</span>
								</span>
								{onDisk && (
									<button type="button" className="control btn-quiet" onClick={() => void revealPath(entry.path)}>
										Reveal
									</button>
								)}
							</div>
						</div>
					</section>
					{entry.source === "gateway" && (
						<section>
							<div className="grouped">
								<div className="group-row">
									<span className="group-row-text">
										<span className="group-row-title">Remove skill</span>
										<span className="group-row-detail">Teammates granted it lose it at their next start.</span>
									</span>
									<button type="button" className="control btn-quiet text-danger" disabled={busy} onClick={onRemove}>
										Remove
									</button>
								</div>
							</div>
						</section>
					)}
					{refusal !== null && <Refusal message={refusal} />}
				</div>
			</Scroll>
		</div>
	);
}
