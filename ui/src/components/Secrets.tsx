import { Fragment, useCallback, useEffect, useState } from "react";
import type { SharedSecret } from "../generated/contract";
import { CloseIcon, PlusIcon, WarningIcon } from "../icons";
import { BackKey, Band } from "../ui/Band";
import { Refusal } from "../ui/Refusal";
import { Scroll } from "../ui/Scroll";
import { wire } from "../wire";

const NESTED = "group-row pl-7";
/** What the desk asks of a value, said here before the desk has to. */
const MIN_VALUE_CHARS = 8;

function reason(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

/**
 * Settings → Secrets: the keys and tokens the operator keeps for teammates'
 * computers to use. Each sits in this machine's keychain under the name
 * that becomes the environment variable, and its value is written once:
 * nothing here, and nothing a teammate has, ever shows it again. Which
 * teammate gets which is decided on that teammate, under its computer, one
 * tick per name. The three commands are desk-seat only.
 */
export function SecretsSection({ onBack }: { onBack?: (() => void) | undefined }) {
	const [stored, setStored] = useState<SharedSecret[] | null>(null);
	const [refusal, setRefusal] = useState<string | null>(null);
	// "" is a new secret; a name is that one being replaced.
	const [editing, setEditing] = useState<string | null>(null);
	const [removing, setRemoving] = useState<string | null>(null);
	const [busy, setBusy] = useState(false);

	const refresh = useCallback(() => {
		void wire
			.command("secrets.list", {})
			.then(setStored)
			.catch((error: unknown) => setRefusal(reason(error)));
	}, []);

	useEffect(() => {
		refresh();
	}, [refresh]);

	const store = async (name: string, value: string) => {
		setBusy(true);
		setRefusal(null);
		try {
			await wire.command("secrets.set", { name, value });
			setEditing(null);
			refresh();
		} catch (error) {
			setRefusal(reason(error));
		} finally {
			setBusy(false);
		}
	};

	const remove = async (name: string) => {
		setBusy(true);
		setRefusal(null);
		try {
			await wire.command("secrets.delete", { name });
			setRemoving(null);
			refresh();
		} catch (error) {
			setRefusal(reason(error));
		} finally {
			setBusy(false);
		}
	};

	return (
		<div className="pane">
			<Band>
				{onBack !== undefined && <BackKey onBack={onBack} />}
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">Secrets</h2>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">
					<section>
						<h3 className="group-title">Stored</h3>
						<div className="grouped">
							{editing === "" ? (
								<SecretForm busy={busy} onStore={store} onCancel={() => setEditing(null)} />
							) : (
								<button
									type="button"
									className="group-row group-row-add"
									disabled={busy}
									onClick={() => {
										setRemoving(null);
										setEditing("");
									}}
								>
									<PlusIcon />
									Store a secret
								</button>
							)}
							{stored === null ? (
								<p className="group-row text-sm text-ink-3">Reading the keychain…</p>
							) : stored.length === 0 ? (
								<p className="group-row text-sm text-ink-3">Nothing stored yet.</p>
							) : (
								stored.map((secret) => (
									<Fragment key={secret.name}>
										<div className="group-row">
											<span className="group-row-text">
												<span className="group-row-title font-mono text-sm">{secret.name}</span>
												<span className="group-row-detail">Stored {new Date(secret.updatedAt).toLocaleDateString()}</span>
											</span>
											<button
												type="button"
												className="control btn-quiet btn-sm"
												disabled={busy}
												onClick={() => {
													setRemoving(null);
													setEditing(secret.name);
												}}
											>
												Replace
											</button>
											<button
												type="button"
												className="control btn-icon -mr-1.5"
												aria-label={`Remove ${secret.name}`}
												title="Remove"
												disabled={busy}
												onClick={() => {
													setEditing(null);
													setRemoving(secret.name);
												}}
											>
												<CloseIcon />
											</button>
										</div>
										{editing === secret.name && (
											<SecretForm name={secret.name} busy={busy} onStore={store} onCancel={() => setEditing(null)} />
										)}
										{removing === secret.name && (
											<div className={`${NESTED} flex-col items-stretch gap-3 py-3`}>
												<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
													Remove {secret.name}? A teammate given it loses it from its computer, and the value is kept nowhere else.
												</span>
												<div className="flex justify-end gap-2">
													<button type="button" className="control btn-quiet" disabled={busy} onClick={() => setRemoving(null)}>
														Cancel
													</button>
													<button type="button" className="control btn" disabled={busy} onClick={() => void remove(secret.name)}>
														Remove
													</button>
												</div>
											</div>
										)}
									</Fragment>
								))
							)}
						</div>
						<p className="group-hint">
							A secret is kept in this machine's keychain under the name that becomes an environment variable in a
							teammate's computer. The value is written once and never shown again, here or to a teammate; the computer
							redacts it from what its tools answer. Give one to a teammate in its pane, under its computer.
						</p>
					</section>
					{refusal !== null && <Refusal message={refusal} />}
				</div>
			</Scroll>
		</div>
	);
}

/** The one form: a name, unless replacing, and a value that is typed once and never read back. */
function SecretForm({
	name: fixed,
	busy,
	onStore,
	onCancel,
}: {
	name?: string;
	busy: boolean;
	onStore(name: string, value: string): Promise<void>;
	onCancel(): void;
}) {
	const [name, setName] = useState(fixed ?? "");
	const [value, setValue] = useState("");
	const ready = name.trim() !== "" && value.length >= MIN_VALUE_CHARS;
	const submit = () => {
		if (!ready || busy) return;
		void onStore(name.trim(), value);
	};

	return (
		<div className={`${NESTED} flex-col items-stretch gap-3 py-3`}>
			<div>
				<label className="label" htmlFor="secret-name">
					Name
				</label>
				<input
					id="secret-name"
					className="field font-mono text-sm"
					placeholder="GITHUB_TOKEN"
					autoComplete="off"
					spellCheck={false}
					readOnly={fixed !== undefined}
					value={name}
					onChange={(event) => setName(event.target.value.toUpperCase())}
				/>
			</div>
			<div>
				<label className="label" htmlFor="secret-value">
					Value
				</label>
				<input
					id="secret-value"
					type="password"
					className="field text-sm"
					autoComplete="new-password"
					spellCheck={false}
					value={value}
					onChange={(event) => setValue(event.target.value)}
					onKeyDown={(event) => {
						if (event.key !== "Enter") return;
						event.preventDefault();
						submit();
					}}
				/>
			</div>
			<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
				{fixed === undefined ? "The name is the environment variable a teammate's computer finds it under. " : ""}
				At least eight characters, one line. Kept in this machine's keychain and never shown again.
			</span>
			<div className="flex justify-end gap-2">
				<button type="button" className="control btn-quiet" disabled={busy} onClick={onCancel}>
					Cancel
				</button>
				<button type="button" className="control btn" disabled={busy || !ready} onClick={submit}>
					{busy ? "Storing…" : fixed === undefined ? "Store" : "Replace"}
				</button>
			</div>
		</div>
	);
}

/**
 * On a teammate, under its computer: the stored names, one tick each. A
 * tick is the grant — nothing is ticked until the person ticks it — and a
 * name the keychain no longer has stays in the list with a warning until
 * it is unticked, so a stale grant is seen rather than silently dropped.
 */
export function ComputerSecrets({
	granted,
	disabled,
	onChange,
}: {
	granted: string[];
	disabled: boolean;
	onChange(granted: string[]): void;
}) {
	const [stored, setStored] = useState<string[] | null>(null);
	const [note, setNote] = useState<string | null>(null);

	useEffect(() => {
		let gone = false;
		wire
			.command("secrets.list", {})
			.then((list) => !gone && setStored(list.map((one) => one.name)))
			.catch((error: unknown) => !gone && setNote(reason(error)));
		return () => {
			gone = true;
		};
	}, []);

	const names = stored === null ? granted : [...stored, ...granted.filter((name) => !stored.includes(name))];
	const toggle = (name: string) =>
		onChange(granted.includes(name) ? granted.filter((one) => one !== name) : [...granted, name].sort());

	return (
		<div className={`${NESTED} flex-col items-stretch gap-1.5 py-3`}>
			<span className="group-row-text">
				<span className="group-row-title">Secrets it can use</span>
				<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
					Each ticked one is an environment variable in every job this computer runs. The teammate is told the names and
					never sees a value. Tick only what its work needs.
				</span>
			</span>
			{stored === null && note === null && <span className="group-row-detail">Reading the keychain…</span>}
			{note !== null && (
				<span className="group-row-detail text-danger" style={{ whiteSpace: "normal" }}>
					{note}
				</span>
			)}
			{stored !== null && names.length === 0 && (
				<span className="group-row-detail" style={{ whiteSpace: "normal" }}>
					Nothing stored yet. Store one under Settings → Secrets.
				</span>
			)}
			{names.map((name) => {
				const missing = stored !== null && !stored.includes(name);
				return (
					<label key={name} className="flex cursor-pointer items-center gap-2 py-1 text-sm">
						<input type="checkbox" className="check" checked={granted.includes(name)} disabled={disabled} onChange={() => toggle(name)} />
						<span className="min-w-0 flex-1 truncate font-mono">{name}</span>
						{missing && (
							<span
								className="flex items-center gap-1 text-danger"
								title="Not stored any more. Untick it, or store it again under Settings → Secrets."
							>
								<WarningIcon />
								<span className="group-row-detail text-danger">not stored</span>
							</span>
						)}
					</label>
				);
			})}
		</div>
	);
}
