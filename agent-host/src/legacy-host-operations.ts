import type { ContextSnapshot } from "./context-manager.ts";
import type { ResourceSnapshot } from "./harness.ts";
import type { ThinkingLevel } from "./policy/tool-policy.ts";
import type { WorkspaceGrantSnapshot } from "./permissions/approvals/workspace-store.ts";
import type { PlanExecutionResult } from "./plan-execution.ts";
import type { PlanArtifact } from "./plan.ts";
import type { QuestionAnswer } from "./questions.ts";
import type {
  SessionBrowserSnapshot,
  TreeFilterMode,
  TreeSnapshot,
} from "./session-navigation.ts";
import type {
  ActiveAgentSnapshot,
  AgentsSnapshot,
  BootstrapState,
} from "./protocol/contracts.ts";
import type { JsonObject } from "./protocol/validation.ts";

export type ApprovalReplyDecision =
  | "allow_once"
  | "allow_session"
  | "allow_workspace"
  | "deny";

export interface LegacyHostOperations {
  listProviders(): Promise<unknown[]>;
  bootstrapState(): BootstrapState;
  startLogin(input: {
    flowId: string;
    providerId: string;
    authType: "oauth" | "api_key";
  }): Promise<{
    providerId: string;
    credentialType: string;
    selectedModel: unknown;
  }>;
  replyToPrompt(input: {
    flowId: string;
    promptId: string;
    value: string;
  }): void;
  cancelLogin(): void;
  logout(providerId: string): Promise<void>;
  setPlanMode(active: boolean): { active: boolean; activeTools: readonly string[] };
  replyQuestion(input: {
    requestId: string;
    answers: QuestionAnswer[];
  }): void;
  planState(): { scopeId: string; artifact: PlanArtifact | null };
  contextSnapshot(): ContextSnapshot;
  resourceSnapshot(): ResourceSnapshot;
  reloadResources(): Promise<ResourceSnapshot>;
  setWorkspaceTrust(trusted: boolean): Promise<ResourceSnapshot>;
  workspaceApprovalRules(): WorkspaceGrantSnapshot;
  revokeApprovalRule(ruleId: string): WorkspaceGrantSnapshot;
  clearApprovalRules(): WorkspaceGrantSnapshot;
  clearQueue(): JsonObject;
  listModels(): Promise<{
    current: { provider: string; id: string } | null;
    models: Array<{
      provider: string;
      id: string;
      name: string;
      reasoning: unknown;
      contextWindow: unknown;
    }>;
  }>;
  setModel(input: {
    provider: string;
    modelId: string;
  }): Promise<{ provider: string; id: string; name: string }>;
  setThinking(level: ThinkingLevel): JsonObject;
  agentsSnapshot(): AgentsSnapshot;
  reloadAgents(): Promise<AgentsSnapshot>;
  startSubagent(input: {
    profile: string;
    task: string;
  }): { accepted: boolean; agent: ActiveAgentSnapshot };
  cancelSubagent(agentId: string): Promise<void>;
  integrateSubagent(input: {
    agentId: string;
    action: "apply" | "resolve" | "keep" | "discard";
  }): Promise<JsonObject>;
  openSessionBrowser(): Promise<SessionBrowserSnapshot>;
  querySessionBrowser(input: {
    browserId: string;
    scope: "current" | "all";
    sortMode: "threaded" | "recent" | "relevance";
    query: string;
    namedOnly: boolean;
    offset: number;
  }): Promise<SessionBrowserSnapshot>;
  closeSessionBrowser(browserId: string): void;
  newSession(): Promise<{ cancelled: boolean; activation?: JsonObject }>;
  resumeSession(input: {
    sessionPath: string;
    cwdOverride?: string;
  }): Promise<{ cancelled: boolean; activation?: JsonObject }>;
  treeState(input: {
    filterMode: TreeFilterMode;
    query: string;
    foldedEntryIds: string[];
  }): TreeSnapshot;
  setTreeLabel(input: { entryId: string; label?: string }): void;
  copyTreeEntry(entryId: string): Promise<void>;
  navigateTree(input: {
    entryId: string;
    summarize: boolean;
    customInstructions?: string;
  }): Promise<JsonObject>;
  abortTreeNavigation(): void;
  executePlan(context: "current" | "fresh"): Promise<PlanExecutionResult>;
  replyApproval(input: {
    requestId: string;
    decision: ApprovalReplyDecision;
  }): void;
}
