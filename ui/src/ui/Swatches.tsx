/**
 * A row of round colour keys, one on: the accent, a teammate's bubble. Each
 * key names its colour in its tooltip and to a screen reader, and is a button
 * that shows itself pressed, as a chip does. A key paints itself through a
 * data attribute tokens.css answers (`data-accent="sky"`), so it shows its own
 * colour in the palette the window is in, whatever colour is chosen.
 */
export function Swatches<T extends string>({
	value,
	choices,
	label,
	attribute,
	onChange,
}: {
	value: T;
	choices: readonly { id: T; name: string }[];
	label: string;
	/** The attribute that names a key's colour to tokens.css. */
	attribute: "data-accent" | "data-bubble";
	onChange(id: T): void;
}) {
	return (
		<fieldset className="swatches" aria-label={label}>
			{choices.map((one) => (
				<button
					key={one.id}
					type="button"
					aria-pressed={value === one.id}
					aria-label={one.name}
					title={one.name}
					className="swatch"
					{...{ [attribute]: one.id }}
					data-on={value === one.id ? "" : undefined}
					onClick={() => onChange(one.id)}
				/>
			))}
		</fieldset>
	);
}
