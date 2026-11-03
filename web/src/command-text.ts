export function visibleText(text: string): string {
  return text
    .replace(/\r\n/g, "⟨CRLF⟩")
    .replace(/\r/g, "⟨CR⟩")
    .replace(/\n/g, "⟨LF⟩")
    .replace(/\t/g, "⟨TAB⟩")
    .replace(
      /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g,
      (c) => `⟨0x${c.charCodeAt(0).toString(16).padStart(2, "0")}⟩`,
    );
}
