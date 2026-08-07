import type {
  FileOperation,
  GrantBundle,
  PermissionAdapter,
  PermissionExplanation,
  PermissionIntent,
  ToolContext,
} from "../model.ts";
import { canonicalizePath } from "../filesystem/path.ts";
import {
  createIntent,
  defaultExplanation,
  exactGrantProposals,
} from "./tool-adapter.ts";

export interface FileToolInput {
  path: string;
  destination?: string;
  [key: string]: unknown;
}

export class FileSystemAdapter implements PermissionAdapter<FileToolInput> {
  private readonly tool: string;
  private readonly operations: readonly FileOperation[];

  constructor(
    tool: string,
    operation: FileOperation | readonly FileOperation[],
  ) {
    this.tool = tool;
    this.operations = Array.isArray(operation) ? operation : [operation];
  }

  normalize(context: ToolContext, input: FileToolInput): PermissionIntent {
    const path = canonicalizePath(context.cwd, input.path);
    const destination = input.destination
      ? canonicalizePath(context.cwd, input.destination)
      : undefined;
    return createIntent(
      context,
      this.tool,
      {
        ...input,
        path,
        operations: this.operations,
        ...(destination ? { destination } : {}),
      },
      this.operations.map((operation) => ({
        kind: "file" as const,
        operation,
        path,
        ...(destination ? { destination } : {}),
      })),
    );
  }

  proposeGrants(intent: PermissionIntent): GrantBundle[] {
    return exactGrantProposals(intent);
  }

  explain(intent: PermissionIntent): PermissionExplanation {
    return defaultExplanation(intent);
  }
}

export const ReadAdapter = new FileSystemAdapter("read", "read");
export const ListAdapter = new FileSystemAdapter("read", "list");
export const WriteAdapter = new FileSystemAdapter(
  "write",
  ["truncate", "write"],
);
export const CreateAdapter = new FileSystemAdapter(
  "write",
  ["create", "write"],
);
export const AppendAdapter = new FileSystemAdapter("write", "append");
export const EditAdapter = new FileSystemAdapter("edit", "write");
export const RenameAdapter = new FileSystemAdapter("rename", "rename");
export const DeleteAdapter = new FileSystemAdapter("delete", "delete");
