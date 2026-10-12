import React from "react";

interface MouthyWordmarkProps {
  className?: string;
}

const MouthyWordmark: React.FC<MouthyWordmarkProps> = ({ className = "" }) => {
  return (
    // eslint-disable-next-line i18next/no-literal-string
    <span className={`font-bold tracking-tight text-text ${className}`}>
      Mouthy
    </span>
  );
};

export default MouthyWordmark;
