/**
 * The options a combo box lists. Opened from the caret or the arrow keys it
 * offers all of them; once the user types, only those that start with what
 * the field holds. A list that would only repeat the field is no list.
 *
 * Kept out of `ComboBox.tsx` so that module exports components only, which is
 * what Fast Refresh needs to keep its state across an edit.
 */
export const comboMatches = (
  options: readonly string[],
  text: string,
  filtered: boolean,
): string[] => {
  if (!filtered) return [...options];
  const prefix = text.toLowerCase();
  const matches = options.filter((option) =>
    option.toLowerCase().startsWith(prefix),
  );
  return matches.length === 1 && matches[0] === text ? [] : matches;
};
