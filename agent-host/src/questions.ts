export interface QuestionOption {
  id: string;
  label: string;
  description?: string;
}

export interface PlanQuestion {
  id: string;
  prompt: string;
  options: QuestionOption[];
}

export interface QuestionAnswer {
  questionId: string;
  value: string;
  optionId?: string;
}

interface PendingQuestion {
  questions: PlanQuestion[];
  resolve(answers: QuestionAnswer[]): void;
  reject(error: Error): void;
}

export class QuestionQueue {
  private nextId = 1;
  private readonly pending = new PendingRequestRegistry<PendingQuestion>();

  request(
    questions: PlanQuestion[],
    signal: AbortSignal | undefined,
    notify: (requestId: string, questions: PlanQuestion[]) => void,
    onCancelled: (requestId: string) => void,
  ): Promise<QuestionAnswer[]> {
    validateQuestions(questions);
    if (signal?.aborted) {
      return Promise.reject(new Error("Question flow cancelled"));
    }

    const requestId = `question-${this.nextId++}`;

    return new Promise<QuestionAnswer[]>((resolve, reject) => {
      const pending: PendingQuestion = {
        questions: structuredClone(questions),
        resolve,
        reject,
      };
      const onAbort = () => {
        const aborted = this.pending.take(requestId);
        if (!aborted) return;
        onCancelled(requestId);
        aborted.reject(new Error("Question flow cancelled"));
      };
      this.pending.register(requestId, pending, () =>
        signal?.removeEventListener("abort", onAbort),
      );
      signal?.addEventListener("abort", onAbort, { once: true });

      // Abort may race with listener registration. Re-check after the request is
      // stored so onAbort can remove it without leaving a stale queue entry.
      if (signal?.aborted) {
        onAbort();
        return;
      }

      try {
        notify(requestId, questions);
      } catch (error) {
        this.pending
          .take(requestId)
          ?.reject(error instanceof Error ? error : new Error(String(error)));
      }
    });
  }

  reply(requestId: string, answers: QuestionAnswer[]): boolean {
    const pending = this.pending.get(requestId);
    if (!pending) return false;
    validateAnswers(answers, pending.questions);
    this.pending.take(requestId);
    pending.resolve(structuredClone(answers));
    return true;
  }

  cancelAll(reason = "Question host stopped"): void {
    for (const pending of this.pending.drain()) {
      pending.reject(new Error(reason));
    }
  }
}

export function validateQuestions(questions: PlanQuestion[]): void {
  if (questions.length < 1 || questions.length > 3) {
    throw new Error("ask_user requires between 1 and 3 questions");
  }
  const questionIds = new Set<string>();
  for (const question of questions) {
    if (!question.id.trim() || !question.prompt.trim()) {
      throw new Error("Question id and prompt must not be empty");
    }
    if (questionIds.has(question.id)) {
      throw new Error(`Duplicate question id: ${question.id}`);
    }
    questionIds.add(question.id);
    if (question.options.length < 2 || question.options.length > 4) {
      throw new Error(`Question ${question.id} requires between 2 and 4 options`);
    }
    const optionIds = new Set<string>();
    for (const option of question.options) {
      if (!option.id.trim() || !option.label.trim()) {
        throw new Error("Option id and label must not be empty");
      }
      if (optionIds.has(option.id)) {
        throw new Error(`Duplicate option id ${option.id} in question ${question.id}`);
      }
      optionIds.add(option.id);
    }
  }
}

function validateAnswers(answers: QuestionAnswer[], questions: PlanQuestion[]): void {
  if (answers.length !== questions.length) {
    throw new Error("question_reply must answer every requested question exactly once");
  }
  const questionsById = new Map(questions.map((question) => [question.id, question]));
  const answered = new Set<string>();
  for (const answer of answers) {
    if (!answer.questionId.trim() || !answer.value.trim()) {
      throw new Error("Answer questionId and value must not be empty");
    }
    const question = questionsById.get(answer.questionId);
    if (!question) throw new Error(`Unknown answered question: ${answer.questionId}`);
    if (answered.has(answer.questionId)) {
      throw new Error(`Duplicate answer for question: ${answer.questionId}`);
    }
    answered.add(answer.questionId);
    if (answer.optionId && !question.options.some((option) => option.id === answer.optionId)) {
      throw new Error(`Unknown option ${answer.optionId} for question ${answer.questionId}`);
    }
  }
}
import { PendingRequestRegistry } from "./protocol/pending-request-registry.ts";
