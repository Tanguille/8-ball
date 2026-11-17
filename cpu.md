# Benchmark: cpu

All models ran on the CPU of the GitHub runner.

Run on 2026-11-17, 8-ball commit 033efb6421f450fe3b1486a3afdee4836997fe38, [run](https://github.com/Tanguille/8-ball/actions/runs/44065632986).

| Method | Decided | Wrong | Accuracy of decided | Correct of all | Correct vs d3 (95% CI) | Accuracy at 90% coverage | Mean time | Speed vs d3 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| d3 (vllm-d3-nano-Q8_0.gguf) | 90% | 10 | 98.2% | 88.8% | reference | 98.3% | 1605 ms | 1.0x |
| rerank (bge-reranker-v2-m3-Q8_0.gguf) | 0% | 0 | - | 0.0% | -88.8 (-91.3 to -86.3) | 55.4% | 164 ms | 9.8x |
| chat (Qwen3.5-0.8B-Q8_0.gguf) | 36% | 31 | 85.5% | 30.5% | -58.3 (-63.0 to -53.8) | 56.1% | 580 ms | 2.8x |

Per task, accuracy of decided (decided share):

| Method | spam | positive | lost-card |
| --- | --- | --- | --- |
| d3 (vllm-d3-nano-Q8_0.gguf) | 100.0% (98%) | 93.8% (81%) | 100.0% (92%) |
| rerank (bge-reranker-v2-m3-Q8_0.gguf) | - (0%) | - (0%) | - (0%) |
| chat (Qwen3.5-0.8B-Q8_0.gguf) | 90.5% (10%) | 77.0% (56%) | 96.2% (40%) |

600 cases per method (yes/no: spam 100/100, positive 100/100, lost-card 100/100), 0 errors. Hardware: AMD EPYC 9V74 80-Core Processor, no GPU, CPU only.

Compared with the baseline (decided, wrong):

| Method | Task | Decided | Wrong | Result |
| --- | --- | --- | --- | --- |
| d3 | spam | 196 (+0) | 0 (+0) | same |
| d3 | positive | 162 (+0) | 10 (+0) | same |
| d3 | lost-card | 185 (+0) | 0 (+0) | same |
| rerank | spam | 0 (+0) | 0 (+0) | same |
| rerank | positive | 0 (+0) | 0 (+0) | same |
| rerank | lost-card | 0 (+0) | 0 (+0) | same |
| chat | spam | 21 (+3) | 2 (+1) | improved |
| chat | positive | 113 (-3) | 26 (-3) | worse (report only) |
| chat | lost-card | 80 (-1) | 3 (+1) | same |
