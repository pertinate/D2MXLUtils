// 3 significant figures with k/M/B suffix. e.g. 1234 → "1.23k", 123400 → "123k".

export function formatDps(value: number): string {
  if (!Number.isFinite(value) || value < 0) return '0';
  if (value < 1000) return Math.round(value).toString();

  const units = ['', 'k', 'M', 'B'];
  let unitIdx = 0;
  let scaled = value;
  while (scaled >= 1000 && unitIdx < units.length - 1) {
    scaled /= 1000;
    unitIdx++;
  }

  const str =
    scaled >= 100 ? scaled.toFixed(0) :
    scaled >= 10  ? scaled.toFixed(1) :
                    scaled.toFixed(2);

  return str + units[unitIdx];
}
