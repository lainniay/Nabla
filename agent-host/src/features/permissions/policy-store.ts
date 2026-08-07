import type { PermissionRule } from "./model.ts";

export class PolicyStore {
  private builtin: PermissionRule[] = [];
  private managed: PermissionRule[] = [];
  private user: PermissionRule[] = [];
  private project: PermissionRule[] = [];

  setBuiltin(rules: readonly PermissionRule[]): void {
    this.builtin = rules.map((rule) => ({ ...rule, source: "builtin" }));
  }

  setManaged(rules: readonly PermissionRule[]): void {
    this.managed = rules.map((rule) => ({ ...rule, source: "managed" }));
  }

  setUser(rules: readonly PermissionRule[]): void {
    this.user = rules.map((rule) => ({ ...rule, source: "user" }));
  }

  setProject(rules: readonly PermissionRule[]): void {
    this.project = rules
      .filter((rule) => rule.effect !== "allow")
      .map((rule) => ({ ...rule, source: "workspace" }));
  }

  all(): PermissionRule[] {
    return [...this.builtin, ...this.managed, ...this.user, ...this.project];
  }
}
