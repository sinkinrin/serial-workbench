export type ColorRule = {
  keyword: string;
  color: string;
  enabled: boolean;
  caseSensitive: boolean;
};
export type ColorSettings = {
  version: 1;
  builtins: boolean;
  rules: ColorRule[];
};
export const COLOR_STORAGE_KEY = "serial-workbench-log-colors-v1";
export const MAX_COLOR_RULES = 100;
export function defaultColors(): ColorSettings {
  return { version: 1, builtins: true, rules: [] };
}
export function parseColors(raw: string | null): ColorSettings {
  if (!raw) return defaultColors();
  try {
    const value = JSON.parse(raw);
    if (value?.version !== 1 || !Array.isArray(value.rules))
      return defaultColors();
    const rules: ColorRule[] = [];
    for (const item of value.rules.slice(0, MAX_COLOR_RULES)) {
      if (!item || typeof item.keyword !== "string" || !item.keyword.trim())
        continue;
      if (typeof item.color !== "string" || !/^#[0-9a-f]{6}$/i.test(item.color))
        continue;
      rules.push({
        keyword: item.keyword.slice(0, 128),
        color: item.color,
        enabled: item.enabled !== false,
        caseSensitive: item.caseSensitive === true,
      });
    }
    return { version: 1, builtins: value.builtins !== false, rules };
  } catch {
    return defaultColors();
  }
}
