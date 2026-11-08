// milsymbol's ES entry (aliased in vite.config.ts): the base and the symbol sets to register.
declare module "milsymbol-esm" {
  import type { Symbol as MilSymbol } from "milsymbol";
  export const ms: { Symbol: typeof MilSymbol; addIcons(set: unknown): void };
  export const std2525c: unknown;
}
