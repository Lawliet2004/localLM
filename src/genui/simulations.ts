export interface SimulationFrame {
  values: number[];
  active: number[];
  message: string;
}
export function binarySearch(
  input: number[],
  target: number,
): SimulationFrame[] {
  const a = [...input].sort((x, y) => x - y);
  const out: SimulationFrame[] = [];
  let lo = 0,
    hi = a.length - 1;
  while (lo <= hi) {
    const mid = Math.floor((lo + hi) / 2);
    out.push({
      values: a,
      active: [lo, mid, hi],
      message: `Range ${lo}–${hi}, middle ${mid}: ${a[mid]}`,
    });
    if (a[mid] === target) {
      out.push({
        values: a,
        active: [mid],
        message: `Found ${target} at index ${mid}.`,
      });
      return out;
    }
    if (a[mid] < target) lo = mid + 1;
    else hi = mid - 1;
  }
  out.push({ values: a, active: [], message: `${target} was not found.` });
  return out;
}
export function sorting(input: number[]): SimulationFrame[] {
  const a = [...input],
    out: SimulationFrame[] = [
      { values: [...a], active: [], message: "Initial values" },
    ];
  for (let i = 1; i < a.length; i++) {
    let j = i;
    while (j > 0 && a[j - 1] > a[j]) {
      [a[j - 1], a[j]] = [a[j], a[j - 1]];
      out.push({
        values: [...a],
        active: [j - 1, j],
        message: `Swap positions ${j - 1} and ${j}`,
      });
      j--;
    }
  }
  out.push({ values: a, active: [], message: "Sorted in ascending order." });
  return out;
}
export function lru(input: number[], capacity: number): SimulationFrame[] {
  const cache: number[] = [];
  return input.map((value) => {
    const at = cache.indexOf(value);
    if (at >= 0) cache.splice(at, 1);
    cache.unshift(value);
    const evicted = cache.length > capacity ? cache.pop() : undefined;
    return {
      values: [...cache],
      active: [0],
      message: `${at >= 0 ? "Hit" : "Miss"}: ${value}${evicted !== undefined ? `; evicted ${evicted}` : ""}. Most recent first.`,
    };
  });
}
export function traversal(
  values: number[],
  edges: [number, number][],
  dfs: boolean,
): SimulationFrame[] {
  const frontier = [0],
    seen = new Set<number>(),
    order: number[] = [],
    out: SimulationFrame[] = [];
  while (frontier.length) {
    const index = dfs ? frontier.pop()! : frontier.shift()!;
    if (index >= values.length || seen.has(index)) continue;
    seen.add(index);
    order.push(values[index]);
    out.push({
      values: [...order],
      active: [order.length - 1],
      message: `Visit node ${index} (${values[index]})`,
    });
    const neighbors = edges.filter((e) => e[0] === index).map((e) => e[1]);
    frontier.push(...(dfs ? neighbors.reverse() : neighbors));
  }
  return out;
}

/** SQL uses bag semantics: duplicate matching rows form a Cartesian product. */
export function sqlJoin(
  left: number[],
  right: number[],
  kind: "inner" | "left" | "full",
): [number | null, number | null][] {
  const result: [number | null, number | null][] = [],
    matched = new Set<number>();
  for (const a of left) {
    let found = false;
    right.forEach((b, i) => {
      if (a === b) {
        result.push([a, b]);
        matched.add(i);
        found = true;
      }
    });
    if (!found && kind !== "inner") result.push([a, null]);
  }
  if (kind === "full")
    right.forEach((b, i) => {
      if (!matched.has(i)) result.push([null, b]);
    });
  return result;
}
