import { asError } from "./protocol/validation.ts";
import type { PlanQuestion } from "./protocol/schemas/questions.ts";
import { RequestQueue } from "./features/interactions/request-queue.ts";

export type {
  PlanQuestion,
  QuestionOption,
} from "./protocol/schemas/questions.ts";

export interface QuestionAnswer {
  questionId: string;
  value: string;
  optionId?: string;
}

export class QuestionQueue {
  private nextId = 1;
  private readonly queue = new RequestQueue<PlanQuestion[], QuestionAnswer[]>();

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

    return this.queue.request(
      requestId,
      structuredClone(questions),
      signal ? [signal] : [],
      () => notify(requestId, questions),
      {
        onAbort: (pending, announced) => {
          if (announced) onCancelled(requestId);
          pending.reject(new Error("Question flow cancelled"));
        },
        onNotifyError: (pending, error) => pending.reject(asError(error)),
      },
    );
  }

  reply(requestId: string, answers: QuestionAnswer[]): boolean {
    const pending = this.queue.get(requestId);
    if (!pending) return false;
    validateAnswers(answers, pending.request);
    return this.queue.reply(requestId, structuredClone(answers));
  }

  cancelAll(reason = "Question host stopped"): void {
    this.queue.settleAll((pending) => pending.reject(new Error(reason)));
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
