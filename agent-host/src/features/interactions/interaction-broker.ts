import {
  ApprovalQueue,
  type ApprovalDecision,
  type ApprovalRequest,
} from "../../approval.ts";
import {
  QuestionQueue,
  type PlanQuestion,
  type QuestionAnswer,
} from "../../questions.ts";

export class InteractionBroker {
  private readonly approvals = new ApprovalQueue();
  private readonly questions = new QuestionQueue();

  requestApproval(
    request: ApprovalRequest,
    signal: AbortSignal | undefined,
    notify: (event: Record<string, unknown>) => void,
  ): Promise<ApprovalDecision> {
    return this.approvals.request(request, signal, notify);
  }

  replyApproval(requestId: string, decision: ApprovalDecision): void {
    if (!this.approvals.reply(requestId, decision)) {
      throw new Error("Approval request is no longer active");
    }
  }

  requestQuestions(
    questions: PlanQuestion[],
    signal: AbortSignal | undefined,
    notify: (requestId: string, questions: PlanQuestion[]) => void,
    onCancelled: (requestId: string) => void,
  ): Promise<QuestionAnswer[]> {
    return this.questions.request(questions, signal, notify, onCancelled);
  }

  replyQuestion(requestId: string, answers: QuestionAnswer[]): void {
    if (!this.questions.reply(requestId, answers)) {
      throw new Error("Question request is no longer active");
    }
  }

  cancelAll(reason = "Host control client disconnected"): void {
    this.approvals.denyAll();
    this.questions.cancelAll(reason);
  }
}
