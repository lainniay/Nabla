import { Type, type Static } from "typebox";
import { Value } from "typebox/value";

import { isJsonObject } from "../validation.ts";
import {
  AgentsSnapshotSchema,
  PendingIntegrationSnapshotSchema,
} from "./agents.ts";
import { ContextSnapshotSchema } from "./context.ts";
import { PlanArtifactSchema } from "./plans.ts";
import { SandboxStatusSchema } from "./sandbox.ts";
import { ResourceSnapshotSchema } from "./workspace.ts";

export const BootstrapStateSchema = Type.Object({
  scopeId: Type.String(),
  planMode: Type.Object({
    active: Type.Boolean(),
    activeTools: Type.Array(Type.String()),
  }),
  sandbox: SandboxStatusSchema,
  plan: Type.Object({
    artifact: Type.Union([Type.Null(), PlanArtifactSchema]),
  }),
  resources: ResourceSnapshotSchema,
  agents: AgentsSnapshotSchema,
  context: ContextSnapshotSchema,
  pendingIntegrations: Type.Array(PendingIntegrationSnapshotSchema),
  warnings: Type.Array(Type.String()),
});

export type BootstrapState = Static<typeof BootstrapStateSchema>;

const DEFAULT_SANDBOX = {
  mode: "disabled",
  backend: "none",
  filesystem: "full-access",
  network: "allowed",
} as const;

export function parseBootstrapState(value: unknown): BootstrapState {
  const candidate =
    isJsonObject(value) && value.sandbox === undefined
      ? { ...value, sandbox: DEFAULT_SANDBOX }
      : value;
  if (!Value.Check(BootstrapStateSchema, candidate)) {
    throw new Error("invalid bootstrap state");
  }
  return Value.Parse(BootstrapStateSchema, candidate) as BootstrapState;
}
