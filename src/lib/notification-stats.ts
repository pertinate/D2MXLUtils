export interface NotificationStatLine {
  line: string;
  originalIndex: number;
}

export function selectNotificationStatLineEntries(
  stats: string,
  matchedStatLines: number[] | null | undefined,
  showOnlyMatchedStats: boolean,
): NotificationStatLine[] {
  const statLines = stats.length > 0 ? stats.split('\n') : [];
  const entries = statLines.map((line, originalIndex) => ({ line, originalIndex }));

  if (!showOnlyMatchedStats || !matchedStatLines || matchedStatLines.length === 0) {
    return entries;
  }

  const selected = matchedStatLines
    .map((index) => entries[index])
    .filter((entry): entry is NotificationStatLine => entry != null);

  return selected.length > 0 ? selected : entries;
}
