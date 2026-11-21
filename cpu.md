# Benchmark: cpu

All models ran on the CPU of the GitHub runner.

Run on 2026-11-21, 8-ball commit 42613ece1d6111b20039c3f095f43f98334ccd6d, [run](https://github.com/Tanguille/8-ball/actions/runs/44949573181).

| Method | Decided | Wrong | Accuracy of decided | Correct of all | Correct vs d3 (95% CI) | Accuracy at 90% coverage | Mean time | Speed vs d3 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| d3 (vllm-d3-nano-Q8_0.gguf) | 90% | 10 | 98.2% | 88.8% | reference | 98.1% | 2466 ms | 1.0x |
| rerank (bge-reranker-v2-m3-Q8_0.gguf) | 0% | 0 | - | 0.0% | -88.8 (-91.3 to -86.3) | 55.4% | 237 ms | 10.4x |
| chat (Qwen3.5-0.8B-Q8_0.gguf) | 35% | 32 | 84.9% | 30.0% | -58.8 (-63.3 to -54.2) | 56.7% | 843 ms | 2.9x |

Per task, accuracy of decided (decided share):

| Method | spam | positive | lost-card |
| --- | --- | --- | --- |
| d3 (vllm-d3-nano-Q8_0.gguf) | 100.0% (98%) | 93.8% (81%) | 100.0% (92%) |
| rerank (bge-reranker-v2-m3-Q8_0.gguf) | - (0%) | - (0%) | - (0%) |
| chat (Qwen3.5-0.8B-Q8_0.gguf) | 95.0% (10%) | 75.4% (57%) | 96.2% (39%) |

600 cases per method (yes/no: spam 100/100, positive 100/100, lost-card 100/100), 0 errors. Hardware: AMD EPYC 7763 64-Core Processor, no GPU, CPU only.

Compared with the baseline (decided, wrong):

| Method | Task | Decided | Wrong | Result |
| --- | --- | --- | --- | --- |
| d3 | spam | 196 (+0) | 0 (+0) | same |
| d3 | positive | 162 (+0) | 10 (+0) | same |
| d3 | lost-card | 185 (+0) | 0 (+0) | same |
| rerank | spam | 0 (+0) | 0 (+0) | same |
| rerank | positive | 0 (+0) | 0 (+0) | same |
| rerank | lost-card | 0 (+0) | 0 (+0) | same |
| chat | spam | 20 (+2) | 1 (+0) | same |
| chat | positive | 114 (-2) | 28 (-1) | same |
| chat | lost-card | 78 (-3) | 3 (+1) | worse (report only) |
