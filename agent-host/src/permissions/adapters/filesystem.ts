import { resolve } from "node:path";

import type {
  FileOperation,
  GrantBundle,
  PermissionAdapter,
  PermissionExplanation,
  PermissionIntent,
  ToolContext,
} from "../model.ts";
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
  private readonly operation: FileOperation;

  constructor(
    tool: string,
    operation: FileOperation,
  ) {
    this.tool = tool;
    this.operation = operation;
  }

  normalize(context: ToolContext, input: FileToolInput): PermissionIntent {
    const path = resolve(context.cwd, input.path);
    const destination = input.destination
      ? resolve(context.cwd, input.destination)
      : undefined;
    return createIntent(
      context,
      this.tool,
      {
        ...input,
        path,
        ...(destination ? { destination } : {}),
      },
      [{
        kind: "file",
        operation: this.operation,
        path,
        ...(destination ? { destination } : {}),
      }],
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
export const WriteAdapter = new FileSystemAdapter("write", "write");
export const CreateAdapter = new FileSystemAdapter("write", "create");
export const AppendAdapter = new FileSystemAdapter("write", "append");
export const EditAdapter = new FileSystemAdapter("edit", "write");
export const RenameAdapter = new FileSystemAdapter("rename", "rename");
export const DeleteAdapter = new FileSystemAdapter("delete", "delete");
