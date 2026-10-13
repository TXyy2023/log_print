export function highlight(text: string, keyword: string): { text: string; match: boolean }[] {
  if (!keyword) return [{ text, match: false }];
  const parts = [];
  let offset = 0;
  let matches = 0;
  for (let at = text.indexOf(keyword); at !== -1 && matches < 256; at = text.indexOf(keyword, offset)) {
    if (at > offset) parts.push({ text: text.slice(offset, at), match: false });
    parts.push({ text: keyword, match: true });
    offset = at + keyword.length;
    matches++;
  }
  if (offset < text.length) parts.push({ text: text.slice(offset), match: false });
  return parts;
}
