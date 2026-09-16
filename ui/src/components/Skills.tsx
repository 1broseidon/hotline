import { useCallback, useEffect, useState } from "react";
import type { SkillEntry } from "../generated/contract";
import { ChevronRightIcon, PlusIcon } from "../icons";
import { pickDirectory, revealPath } from "../native";
import { BackKey, Band } from "../ui/Band";
import { Refusal } from "../ui/Refusal";
import { Scroll } from "../ui/Scroll";
import { wire } from "../wire";

/** One word for where a skill came from, as the row's second line says it. */
export function skillSourceName(entry: SkillEntry): string {
	switch (entry.source) {
		case "builtin":
			return "Built in";
		case "gateway":
			return "Gateway";
		case "workspace":
			return "Workspace";
		case "computer":
			return "Computer";
	}
}

/**
 * Skills: the built-ins and the gateway, drawn the way Tools is. An add row
 * picks a folder and copies it in; a row opens the skill's page, where a
 * gateway skill is removed at the foot. An invalid entry stays in the list
 * with its reason, so the person can fix the folder rather than wonder
 * where it went. Which teammate may read a skill is a different question,
 * answered on that teammate.
 */
export function SkillsSection({ onBack }: { onBack?: (() => void) | undefined }) {
	const [entries, setEntries] = useState<SkillEntry[]>([]);
	const [refusal, setRefusal] = useState<string | null>(null);
	const [open, setOpen] = useState<string | null>(null);
	const [busy, setBusy] = useState(false);

	const refresh = useCallback(() => {
		void wire
			.command("skills.list", {})
			.then(setEntries)
			.catch((error: Error) => setRefusal(error.message));
	}, []);

	useEffect(() => {
		refresh();
	}, [refresh]);

	const add = async () => {
		const path = await pickDirectory();
		if (path === null) return;
		setBusy(true);
		setRefusal(null);
		try {
			await wire.command("skills.add", { path });
			refresh();
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	const remove = async (name: string) => {
		setBusy(true);
		setRefusal(null);
		try {
			await wire.command("skills.remove", { name });
			setOpen(null);
			refresh();
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

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
							A skill is a folder holding SKILL.md with a name and a description. Grant skills per teammate, in its pane.
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
								{entry.source === "gateway" && (
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
