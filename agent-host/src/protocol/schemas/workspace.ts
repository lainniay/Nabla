import { Type, type Static } from "typebox";

export const ResourceSnapshotSchema = Type.Object({
  scopeId: Type.Optional(Type.String()),
  trusted: Type.Boolean(),
  contextFiles: Type.Array(Type.String()),
  skills: Type.Array(
    Type.Object({
      name: Type.String(),
      path: Type.String(),
      description: Type.String(),
    }),
  ),
  prompts: Type.Array(
    Type.Object({
      name: Type.String(),
      path: Type.String(),
      description: Type.String(),
    }),
  ),
  extensions: Type.Array(Type.String()),
  commands: Type.Array(
    Type.Object({
      name: Type.String(),
      description: Type.String(),
      source: Type.Union([
        Type.Literal("extension"),
        Type.Literal("prompt"),
        Type.Literal("skill"),
      ]),
    }),
  ),
  diagnostics: Type.Array(
    Type.Object({
      type: Type.String(),
      message: Type.String(),
      path: Type.Optional(Type.String()),
    }),
  ),
  revision: Type.Number(),
});

export type ResourceSnapshot = Static<typeof ResourceSnapshotSchema>;
