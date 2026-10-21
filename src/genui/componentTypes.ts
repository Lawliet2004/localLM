import { createElement, type ComponentType, type ReactNode } from "react";
import type { ComponentRenderProps } from "@openuidev/react-lang";
import type { z } from "zod/v4";
export type Props<T extends z.ZodType> = { props: z.infer<T> };
export function adapt<T extends z.ZodType>(
  schema: T,
  Component: ComponentType<Props<T>>,
): (input: ComponentRenderProps<Record<string, unknown>>) => ReactNode {
  return ({ props }) => {
    const parsed = schema.safeParse(props);
    return parsed.success
      ? createElement(Component, { props: parsed.data })
      : null;
  };
}
