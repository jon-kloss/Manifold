// WebLLM engine worker: the on-device model (WebGPU) runs HERE, off the main
// thread, so download/compile/generation never freeze the UI. Instantiated only
// when the user opts in (see ai/webllm.ts) — this whole chunk, and the web-llm
// library it statically imports, are lazy.

import { WebWorkerMLCEngineHandler } from "@mlc-ai/web-llm";

const handler = new WebWorkerMLCEngineHandler();
self.onmessage = (msg: MessageEvent) => {
  handler.onmessage(msg);
};
