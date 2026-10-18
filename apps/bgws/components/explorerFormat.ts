/**
 * A figure with its unit, or "—" when there is none. The explorer's detail
 * rows render "—" muted, so an absence reads as an absence rather than as a
 * zero.
 */
export function fmt(value: number | string | null | undefined, suffix = ""): string {
  return value == null || value === "" ? "—" : `${value}${suffix}`;
}
