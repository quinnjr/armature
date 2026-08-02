/**
 * The FAQ, as data.
 *
 * Single source of truth for both the rendered `/faq` page and the `FAQPage`
 * JSON-LD. Google's structured-data policy requires FAQ markup to match the
 * question-and-answer text visible on the page; when the two were maintained
 * separately they drifted — the markup advertised a question the page did not
 * ask, and the page asked two the markup did not declare. Deriving both from
 * this array makes that drift impossible.
 *
 * `answer` is plain text on purpose: it is emitted verbatim into JSON-LD, and
 * answer engines quote it directly.
 */
export interface FaqEntry {
  question: string;
  answer: string;
}

export const faqs: FaqEntry[] = [
  {
    question: 'Is Armature a good NestJS alternative for Rust?',
    answer:
      "Yes. Armature is designed to bring NestJS's architectural patterns to Rust. It features dependency injection, decorators (via Rust macros), modules, guards, interceptors, and pipes — all concepts familiar to NestJS developers.",
  },
  {
    question: 'How does Armature compare to Express or Koa?',
    answer:
      "Armature provides Express's simplicity and Koa's async/await patterns, but adds type safety, dependency injection, and compile-time error catching. The middleware system will feel familiar, and you additionally get Rust's performance and memory-safety guarantees.",
  },
  {
    question: 'Why choose Armature over Actix-web, Rocket, or Axum?',
    answer:
      'Armature focuses on enterprise features and developer experience. Actix-web, Rocket, and Axum are excellent lower-level frameworks; Armature adds built-in authentication, validation, caching, job queues, and OpenAPI generation, reducing the number of external crates a production service has to assemble itself.',
  },
  {
    question: 'Does Armature support GraphQL?',
    answer:
      "Yes. Armature includes GraphQL support with schema-first or code-first approaches, similar to NestJS's GraphQL module, including subscriptions over WebSocket.",
  },
  {
    question: "What's the learning curve coming from Node.js?",
    answer:
      'If you know NestJS or Express, the concepts transfer directly. You will need to learn Rust syntax and ownership, but the architectural patterns — controllers, injectable services, modules, middleware — are the same ones you already use.',
  },
  {
    question: 'Is Armature production ready?',
    answer:
      "Armature is pre-1.0 and its public API can still change between minor versions. The framework is developed against a full workspace test suite and a strict Clippy gate, and its HTTP/1.1 layer carries allocation-regression and differential fuzzing tests. Pin an exact version and read each crate's CHANGELOG before upgrading.",
  },
  {
    question: 'What Rust version does Armature require?',
    answer:
      'Armature targets the Rust 2024 edition with a minimum supported Rust version of 1.94.1. Async support is built on Tokio and Hyper.',
  },
  {
    question: 'How is Armature licensed?',
    answer: 'Armature is released under the Apache License 2.0, so it is free for commercial and private use.',
  },
];
