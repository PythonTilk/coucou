// What can be done with a file once Mochi has swallowed it. The same list
// draws the buttons on the drop card and decides what each one does.

export type UploadChoice = "ask" | "summarise" | "explain" | "quiz" | "cancel";

/** In the order they are shown; the first is the primary button. `w` is its width on the card. */
export const UPLOAD_CHOICES: { id: UploadChoice; label: string; w: number }[] = [
  { id: "ask", label: "Ask", w: 56 },
  { id: "summarise", label: "Summarise", w: 86 },
  { id: "explain", label: "Explain", w: 70 },
  { id: "quiz", label: "Quiz me", w: 74 },
  { id: "cancel", label: "Cancel", w: 66 },
];

/** A question asked on the user's behalf: what the chat shows, and what is sent. */
export interface PresetPrompt {
  label: string;
  query: string;
}

export const STUDY_PROMPTS: Partial<Record<UploadChoice, PresetPrompt>> = {
  summarise: {
    label: "Summarise this for studying",
    query:
      "Summarise this file for studying: the key points, the definitions worth knowing, and anything likely to come up in an exam. Keep it compact.",
  },
  explain: {
    label: "Explain this simply",
    query:
      "Explain the main ideas of this file simply, as you would to a first-year student, with one concrete example for each idea.",
  },
  quiz: {
    label: "Quiz me on this",
    query:
      "Quiz me on this file. Ask five questions, one at a time: wait for my answer, tell me whether it was right, explain briefly, then ask the next one.",
  },
};
