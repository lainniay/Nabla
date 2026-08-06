import {
  type CommandDefinition,
  requestObject,
} from "../command-definition.ts";
import { isJsonObject, stringField } from "../validation.ts";
import type { QuestionAnswer } from "../../questions.ts";

export interface InteractionCommandPort {
  replyApproval(
    requestId: string,
    decision: "allow_once" | "allow_session" | "allow_workspace" | "deny",
  ): void;
  replyQuestion(requestId: string, answers: QuestionAnswer[]): void;
}

export function createInteractionCommands(
  ops: InteractionCommandPort,
): CommandDefinition<any>[] {
  return [
    {
      type: "question_reply",
      lane: undefined,
      decode: (value) => {
        const request = requestObject(value);
        const rawAnswers = request.answers;
        if (!Array.isArray(rawAnswers)) {
          throw new Error("question_reply requires answers");
        }
        const answers = rawAnswers.map((answer) => {
          if (!isJsonObject(answer)) throw new Error("Invalid question answer");
          const optionId =
            typeof answer.optionId === "string" && answer.optionId.length > 0
              ? answer.optionId
              : undefined;
          return {
            questionId: stringField(answer, "questionId"),
            value: stringField(answer, "value"),
            ...(optionId ? { optionId } : {}),
          };
        });
        return {
          requestId: stringField(request, "requestId"),
          answers,
        };
      },
      handle: (_context, request) =>
        ops.replyQuestion(request.requestId, request.answers),
    },
    {
      type: "approval_reply",
      lane: undefined,
      decode: (value) => {
        const request = requestObject(value);
        const decision = stringField(request, "decision");
        if (
          decision !== "allow_once" &&
          decision !== "allow_session" &&
          decision !== "allow_workspace" &&
          decision !== "deny"
        ) {
          throw new Error(`Unsupported approval decision: ${decision}`);
        }
        return {
          requestId: stringField(request, "requestId"),
          decision,
        };
      },
      handle: (_context, request) =>
        ops.replyApproval(request.requestId, request.decision),
    },
  ];
}
