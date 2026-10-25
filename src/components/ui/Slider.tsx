import React, { useEffect, useRef } from "react";
import { SettingContainer } from "./SettingContainer";
import { ResetButton } from "./ResetButton";

interface SliderProps {
  value: number;
  onChange: (value: number) => void;
  min: number;
  max: number;
  step?: number;
  disabled?: boolean;
  label: string;
  description: string;
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  showValue?: boolean;
  formatValue?: (value: number) => string;
  onReset?: () => void;
  isResetting?: boolean;
}

interface RangeInputProps {
  value: number;
  onChange: (value: number) => void;
  /** Called once the value has changed and the user let go, not per step. */
  onCommit?: (value: number) => void;
  min: number;
  max: number;
  step: number;
  disabled?: boolean;
  className?: string;
}

export const RangeInput: React.FC<RangeInputProps> = ({
  value,
  onChange,
  onCommit,
  min,
  max,
  step,
  disabled = false,
  className = "",
}) => {
  const fill = ((value - min) / (max - min)) * 100;
  const input = useRef<HTMLInputElement>(null);
  const latestCommit = useRef(onCommit);
  latestCommit.current = onCommit;

  // React's onChange is the per-step `input` event. The native `change` event
  // fires only for a new value, once released (even outside the control), so
  // focusing the slider or clicking its thumb commits nothing.
  useEffect(() => {
    const element = input.current;
    if (!element) return;
    const commit = () => latestCommit.current?.(parseFloat(element.value));
    element.addEventListener("change", commit);
    return () => element.removeEventListener("change", commit);
  }, []);

  return (
    <input
      ref={input}
      type="range"
      min={min}
      max={max}
      step={step}
      value={value}
      onChange={(e) => onChange(parseFloat(e.target.value))}
      disabled={disabled}
      className={`flex-grow h-2 rounded-lg appearance-none cursor-pointer focus:outline-none focus:ring-2 focus:ring-logo-primary disabled:opacity-50 disabled:cursor-not-allowed ${className}`}
      style={{
        background: `linear-gradient(to right, var(--color-background-ui) ${fill}%, rgba(128, 128, 128, 0.2) ${fill}%)`,
      }}
    />
  );
};

export const Slider: React.FC<SliderProps> = ({
  value,
  onChange,
  min,
  max,
  step = 0.01,
  disabled = false,
  label,
  description,
  descriptionMode = "tooltip",
  grouped = false,
  showValue = true,
  formatValue = (v) => v.toFixed(2),
  onReset,
  isResetting = false,
}) => {
  return (
    <SettingContainer
      title={label}
      description={description}
      descriptionMode={descriptionMode}
      grouped={grouped}
      layout="horizontal"
      disabled={disabled}
    >
      <div className="w-full">
        <div className="flex items-center space-x-1 h-6">
          <RangeInput
            value={value}
            onChange={onChange}
            min={min}
            max={max}
            step={step}
            disabled={disabled}
          />
          {showValue && (
            <span className="text-sm font-medium text-text/90 w-12 text-end">
              {formatValue(value)}
            </span>
          )}
          {onReset && (
            <ResetButton onClick={onReset} disabled={disabled || isResetting} />
          )}
        </div>
      </div>
    </SettingContainer>
  );
};
