import { Type, type Static } from "typebox";

export const SandboxStatusSchema = Type.Object({
  mode: Type.Union([
    Type.Literal("enforced"),
    Type.Literal("degraded"),
    Type.Literal("disabled"),
  ]),
  backend: Type.Union([
    Type.Literal("bubblewrap"),
    Type.Literal("seatbelt"),
    Type.Literal("none"),
  ]),
  filesystem: Type.Union([
    Type.Literal("workspace-write"),
    Type.Literal("full-access"),
  ]),
  network: Type.Union([Type.Literal("blocked"), Type.Literal("allowed")]),
  reason: Type.Optional(Type.String()),
});

export type SandboxStatus = Static<typeof SandboxStatusSchema>;
