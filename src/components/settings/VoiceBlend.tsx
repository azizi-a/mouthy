import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { VoicePart } from "@/bindings";
import { useSettings } from "../../hooks/useSettings";
import { Button } from "../ui/Button";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import { RangeInput } from "../ui/Slider";
import { PreviewButton, PreviewNotice } from "./VoiceRow";

const DEFAULT_WEIGHT = 70;

/** A share as the slider's 10–90 in steps of 10. */
const toWeight = (share: number) =>
  Math.min(90, Math.max(10, Math.round(share * 10) * 10));

interface VoiceBlendProps {
  /** The voice in use, or each voice of the blend in use. */
  parts: VoicePart[];
  /** Voices that can be the second voice. */
  candidates: string[];
  onEditOther: () => void;
}

/** Blends the voice in use with one other, the first voice keeping its accent. */
export const VoiceBlend: React.FC<VoiceBlendProps> = ({
  parts,
  candidates,
  onEditOther,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const voice = getSetting("tts_voice") ?? "";
  const first = parts[0]?.id ?? "";
  const live = parts.length === 2;
  const [second, setSecond] = useState(live ? parts[1].id : "");
  const [weight, setWeight] = useState(
    live ? toWeight(parts[0].share) : DEFAULT_WEIGHT,
  );

  // Follow the setting when it changes elsewhere.
  const partsKey = parts.map(({ id, share }) => `${id}:${share}`).join("+");
  useEffect(() => {
    if (parts.length === 2) {
      setSecond(parts[1].id);
      setWeight(toWeight(parts[0].share));
    } else {
      setSecond((current) => (current === first ? "" : current));
    }
  }, [partsKey]);

  if (!first) return null;

  const specFor = (w: number, other: string) =>
    `${first}(${w})+${other}(${100 - w})`;
  const spec = second ? specFor(weight, second) : "";
  // A blend in use changes as it is edited; a new one waits for Use.
  const write = (next: string) => {
    if (live && next !== voice) updateSetting("tts_voice", next);
  };

  if (parts.length > 2) {
    return (
      <SettingContainer
        title={t("settings.speech.voice.blend.title")}
        description={t("settings.speech.voice.blend.description")}
        grouped={true}
      >
        <div className="flex items-center gap-2 min-w-0">
          <code
            className="text-xs font-mono truncate max-w-[360px]"
            title={voice}
          >
            {voice}
          </code>
          <Button variant="secondary" size="sm" onClick={onEditOther}>
            {t("settings.speech.voice.blend.editInOther")}
          </Button>
        </div>
      </SettingContainer>
    );
  }

  const options = candidates
    .filter((id) => id !== first)
    .map((id) => ({ value: id, label: id }));

  return (
    <SettingContainer
      title={t("settings.speech.voice.blend.title")}
      description={t("settings.speech.voice.blend.description")}
      grouped={true}
      layout="stacked"
    >
      <div className="flex flex-col gap-2">
        <div className="flex items-center gap-2">
          <span className="text-xs text-mid-gray shrink-0">
            {t("settings.speech.voice.blend.with", { voice: first })}
          </span>
          <Dropdown
            options={options}
            selectedValue={second || null}
            onSelect={(id) => {
              setSecond(id);
              write(specFor(weight, id));
            }}
            placeholder={
              options.length > 0
                ? t("settings.speech.voice.blend.choose")
                : t("settings.speech.voice.blend.none")
            }
            disabled={options.length === 0 || isUpdating("tts_voice")}
          />
        </div>
        <div className="flex items-center gap-2">
          <span className="text-xs tabular-nums w-9 text-end">{weight}%</span>
          <RangeInput
            value={weight}
            onChange={setWeight}
            onCommit={(w) => {
              // A stored 75/25 shows as 80; committing the shown weight
              // unchanged must not round the typed blend.
              const unchanged =
                live &&
                second === parts[1].id &&
                w === toWeight(parts[0].share);
              if (second && !unchanged) write(specFor(w, second));
            }}
            min={10}
            max={90}
            step={10}
            disabled={!second || isUpdating("tts_voice")}
          />
          <span className="text-xs tabular-nums w-9">{100 - weight}%</span>
        </div>
        {second && (
          <div className="flex items-center justify-between gap-2">
            <code
              className="text-xs font-mono truncate max-w-[360px]"
              title={spec}
            >
              {spec}
            </code>
            <div className="flex items-center gap-1 shrink-0">
              <PreviewButton voice={spec} />
              {spec !== voice && (
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={isUpdating("tts_voice")}
                  onClick={() => updateSetting("tts_voice", spec)}
                >
                  {t("settings.models.use")}
                </Button>
              )}
            </div>
          </div>
        )}
        {second && first[0] !== second[0] && (
          <p className="text-xs text-mid-gray">
            {t("settings.speech.voice.blend.accent")}
          </p>
        )}
        {second && <PreviewNotice voice={spec} />}
      </div>
    </SettingContainer>
  );
};
