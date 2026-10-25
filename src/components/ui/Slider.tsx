import React from "react";
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
  /** Called once the user lets go, not at every step on the way. */
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
  const commit = (e: React.SyntheticEvent<HTMLInputElement>) =>
    onCommit?.(parseFloat(e.currentTarget.value));
  return (
    <input
      type="range"
      min={min}
      max={max}
      step={step}
      value={value}
      onChange={(e) => onChange(parseFloat(e.target.value))}
      onPointerUp={commit}
      onKeyUp={commit}
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
