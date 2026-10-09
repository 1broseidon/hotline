/** A short set of choices in a row, one on: the theme, a server's type, how it signs in. */
export function Chips<T extends string>({
	value,
	choices,
	label,
	disabled,
	onChange,
}: {
	value: T;
	choices: readonly { id: T; name: string; title?: string }[];
	label: string;
	disabled?: boolean;
	onChange(id: T): void;
}) {
	return (
		<div className="chips" role="radiogroup" aria-label={label}>
			{choices.map((one) => (
				<button
					key={one.id}
					type="button"
					role="radio"
					aria-checked={value === one.id}
					className="nt-chip"
					data-on={value === one.id ? "" : undefined}
					title={one.title}
					disabled={disabled}
					onClick={() => onChange(one.id)}
				>
					{one.name}
				</button>
			))}
		</div>
	);
}
