import { useEffect, useState } from "react";

interface Props {
  value: number;
  onChange: (v: number) => void;
  label: string;
  className?: string;
}

const round = (x: number) => (Number.isFinite(x) ? Number(x.toPrecision(5)) : 0);

/** A number input that keeps what is being typed (like "1." or "-") and commits every valid number. */
export function NumField({ value, onChange, label, className }: Props) {
  const [text, setText] = useState(String(round(value)));
  const [focus, setFocus] = useState(false);
  useEffect(() => {
    if (!focus) setText(String(round(value)));
  }, [value, focus]);
  return (
    <input
      type="text"
      inputMode="decimal"
      className={`num ${className ?? ""}`}
      aria-label={label}
      value={text}
      onFocus={() => setFocus(true)}
      onBlur={() => {
        setFocus(false);
        setText(String(round(value)));
      }}
      onChange={(e) => {
        setText(e.target.value);
        const v = parseFloat(e.target.value.replace(",", "."));
        if (Number.isFinite(v)) onChange(v);
      }}
    />
  );
}
