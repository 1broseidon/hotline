import { useEffect, useState } from "react";
import type { CatalogModel } from "../../generated/contract";
import { Chips } from "../../ui/Chips";
import { Fold } from "../../ui/Fold";
import { Refusal } from "../../ui/Refusal";
import { wire } from "../../wire";

/**
 * Which of a provider's models the pickers show. Every model checked is the
 * same as no filter, so Save removes the provider's entry rather than
 * writing a list that means "all".
 */
export function ProviderModels({
	providerId,
	discover,
	custom,
	enabledModels,
}: {
	providerId: string;
	discover: boolean;
	custom: boolean;
	enabledModels: Record<string, string[]>;
}) {
	const [refusal, setRefusal] = useState<string | null>(null);
	const [catalog, setCatalog] = useState<CatalogModel[] | null>(null);
	const [manualId, setManualId] = useState("");
	const [notice, setNotice] = useState<string | null>(null);
	const [query, setQuery] = useState("");
	const [on, setOn] = useState<Set<string>>(new Set());
	const [busy, setBusy] = useState(false);
	// A filter is a choice of some models; no filter is every model.
	const [some, setSome] = useState(enabledModels[providerId] !== undefined);
	// Whether the switches hold a choice of the person's, saved or made here.
	// Without one, Chosen starts with every switch off.
	const [picked, setPicked] = useState(enabledModels[providerId] !== undefined);
	const [adding, setAdding] = useState(false);

	const applyCatalog = (list: CatalogModel[], preserveSelections = false) => {
		setCatalog(list);
		setOn((known) => {
			if (!preserveSelections) return new Set([...(enabledModels[providerId] ?? []), ...list.filter((model) => model.enabled).map((model) => model.id)]);
			const next = new Set(known);
			const previousIds = new Set(catalog?.map((model) => model.id));
			for (const model of list) {
				if (!previousIds.has(model.id) && model.enabled) next.add(model.id);
			}
			return next;
		});
	};

	// biome-ignore lint/correctness/useExhaustiveDependencies: the catalog is read once per provider, and applyCatalog reads that render's filter.
	useEffect(() => {
		wire
			.command("models.catalog", { providerId })
			.then((list) => applyCatalog(list))
			.catch((error: Error) => {
				setRefusal(error.message);
				setCatalog(null);
			});
	}, [providerId]);

	const run = async (work: () => Promise<void>) => {
		if (busy) return;
		setBusy(true);
		setRefusal(null);
		setNotice(null);
		try {
			await work();
		} catch (error) {
			setRefusal(error instanceof Error ? error.message : String(error));
		} finally {
			setBusy(false);
		}
	};

	const refresh = () =>
		run(async () => {
			applyCatalog(await wire.command("credential.refresh_models", { providerId }), true);
			setNotice("Model list refreshed. Your choice is unchanged.");
		});

	const setManualModels = async (modelIds: string[]) => {
		const list = await wire.command("models.manual_set", { providerId, modelIds });
		applyCatalog(list, true);
		return list;
	};

	const addManualModel = () => run(async () => {
		const id = manualId.trim();
		if (!id || catalog === null) return;
		const list = await setManualModels([...new Set([...catalog.filter((model) => model.manual).map((model) => model.id), id])]);
		setManualId("");
		setNotice(list.find((model) => model.id === id)?.enabled ? "Model added." : "Model added. Switch it on under Chosen to use it.");
	});

	const removeManualModel = (id: string) => run(async () => {
		if (catalog === null) return;
		await setManualModels(catalog.filter((model) => model.manual && model.id !== id).map((model) => model.id));
	});

	/* Every change is written at once: no filter for every model, else the ones switched on. */
	const write = (nextSome: boolean, nextOn: Set<string>) =>
		run(async () => {
			if (catalog === null) return;
			const next: Record<string, string[]> = { ...enabledModels };
			if (!nextSome) delete next[providerId];
			else next[providerId] = [...nextOn];
			await wire.command("settings.update", { patch: { enabledModels: next } });
		});

	const choose = (nextSome: boolean) => {
		const nextOn = nextSome && !picked ? new Set<string>() : on;
		setSome(nextSome);
		setOn(nextOn);
		if (nextSome) setPicked(true);
		void write(nextSome, nextOn);
	};

	const flipAll = (ids: string[], value: boolean) => {
		const next = new Set(on);
		for (const id of ids) {
			if (value) next.add(id);
			else next.delete(id);
		}
		setOn(next);
		void write(true, next);
	};

	const flip = (id: string, value: boolean) => flipAll([id], value);

	const needle = query.trim().toLowerCase();
	// The ones in the picker lead, in the order they were when the list came;
	// a switch flipped now does not move its row out from under the pointer.
	const [lead] = useState(() => new Set(enabledModels[providerId] ?? []));
	const listed = catalog === null ? [] : [...catalog].sort((a, b) => Number(lead.has(b.id)) - Number(lead.has(a.id)));
	const visible =
		needle === ""
			? listed
			: listed.filter((model) => model.id.toLowerCase().includes(needle) || model.name.toLowerCase().includes(needle));
	const manual = catalog?.filter((model) => model.manual) ?? [];

	return (
		<>
			<section>
				<h3 className="group-title">Models in the picker</h3>
				<div className="grouped">
					<div className="group-row">
						<span className="group-row-text">
							<span className="group-row-title">
								{catalog === null
									? refusal
										? "Model list unavailable"
										: "Reading…"
									: catalog.length === 0 && providerId === "ollama"
										? "No models yet. Pull one with Ollama, then refresh."
										: some
											? `${catalog.filter((model) => on.has(model.id)).length} of ${catalog.length}`
											: `All ${catalog.length}`}
							</span>
						</span>
						{discover && (
							<button type="button" className="control btn-quiet btn-sm" disabled={busy} aria-label="Refresh provider models" onClick={() => void refresh()}>
								Refresh
							</button>
						)}
						<Chips
							value={some ? "some" : "all"}
							choices={[
								{ id: "all", name: "All" },
								{ id: "some", name: "Chosen" },
							]}
							label="Which models are in the picker"
							disabled={busy || catalog === null}
							onChange={(id) => choose(id === "some")}
						/>
					</div>
					{some && catalog !== null && catalog.length > 0 && (
						<div className="group-row">
							{catalog.length > 6 ? (
								<input
									type="search"
									className="field min-w-0 flex-1"
									placeholder="Search models"
									value={query}
									onChange={(event) => setQuery(event.target.value)}
									aria-label="Search models"
								/>
							) : (
								<span className="flex-1" />
							)}
							{/* On a search these act on the models it shows. */}
							<button
								type="button"
								className="control btn-quiet btn-sm"
								disabled={busy || visible.every((model) => on.has(model.id))}
								onClick={() => flipAll(visible.map((model) => model.id), true)}
							>
								All on
							</button>
							<button
								type="button"
								className="control btn-quiet btn-sm"
								disabled={busy || visible.every((model) => !on.has(model.id))}
								onClick={() => flipAll(visible.map((model) => model.id), false)}
							>
								All off
							</button>
						</div>
					)}
					{some &&
						visible.map((model) => (
							<label key={model.id} className="group-row group-row-choice">
								<span className="group-row-text">
									<span className="group-row-title">{model.name}</span>
									<span className="group-row-detail">
										<span className="font-mono">{model.id}</span>
										{tokensText(model) !== "" && ` · ${tokensText(model)}`}
										{model.manual && " · added by you"}
									</span>
								</span>
								<input
									type="checkbox"
									role="switch"
									className="switch"
									aria-label={model.name}
									checked={on.has(model.id)}
									disabled={busy}
									onChange={(event) => flip(model.id, event.target.checked)}
								/>
							</label>
						))}
					{!custom && (
						<Fold
							title="A model by ID"
							value={manual.length === 0 ? "For one this list is missing" : manual.map((model) => model.id).join(", ")}
							action="Add"
							open={adding}
							onToggle={() => setAdding((was) => !was)}
						>
							<form
								className="flex gap-2"
								onSubmit={(event) => {
									event.preventDefault();
									void addManualModel();
								}}
							>
								<input
									className="field min-w-0 flex-1 font-mono text-sm"
									aria-label="Model ID to add"
									placeholder="Exact model ID"
									value={manualId}
									onChange={(event) => setManualId(event.target.value)}
									spellCheck={false}
									autoComplete="off"
									disabled={busy || catalog === null}
								/>
								<button type="submit" className="control btn" disabled={busy || catalog === null || !manualId.trim()}>
									Add
								</button>
							</form>
							{manual.map((model) => (
								<div className="flex items-center gap-2" key={model.id}>
									<span className="min-w-0 flex-1 truncate font-mono text-sm">{model.id}</span>
									<button type="button" className="control btn-quiet btn-sm" disabled={busy} aria-label={`Remove manual model ${model.id}`} onClick={() => void removeManualModel(model.id)}>
										Remove
									</button>
								</div>
							))}
							<p className="hint">
								{providerId === "github-copilot"
									? "It must also be in your Copilot account's model list."
									: "Use one that supports chat and tools."}
							</p>
						</Fold>
					)}
				</div>
			</section>
			{notice !== null && (
				<p className="group-hint" role="status">
					{notice}
				</p>
			)}
			{refusal !== null && <Refusal message={refusal} />}
		</>
	);
}

/** A model's limits in a few characters: "1M context · 128K out". */
function tokensText(model: CatalogModel): string {
	const short = (n: number) => (n >= 1_000_000 ? `${+(n / 1_000_000).toFixed(1)}M` : n >= 1_000 ? `${Math.round(n / 1_000)}K` : String(n));
	return [model.contextLimit ? `${short(model.contextLimit)} context` : null, model.outputLimit ? `${short(model.outputLimit)} out` : null]
		.filter(Boolean)
		.join(" · ");
}
