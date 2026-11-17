/// <reference types="vite/client" />

declare module "bun:test" {
  export function describe(name: string, fn: () => void): void;
  export function test(name: string, fn: () => void | Promise<void>): void;
  export function it(name: string, fn: () => void | Promise<void>): void;
  export function beforeEach(fn: () => void | Promise<void>): void;

  interface Matchers<T> {
    toBe(expected: T): void;
    toEqual(expected: unknown): void;
    toStrictEqual(expected: unknown): void;
    toBeCloseTo(expected: number, numDigits?: number): void;
    toBeGreaterThan(expected: number): void;
    toBeGreaterThanOrEqual(expected: number): void;
    toBeLessThan(expected: number): void;
    toBeLessThanOrEqual(expected: number): void;
    toContain(expected: unknown): void;
    toMatch(pattern: RegExp | string): void;
    toBeTruthy(): void;
    toBeFalsy(): void;
    toBeNull(): void;
    toHaveLength(expected: number): void;
    not: Matchers<T>;
  }

  export function expect<T>(actual: T): Matchers<T>;
}
