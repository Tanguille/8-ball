# Benchmark: cpu

All models ran on the CPU of the GitHub runner.

Run on 2026-11-17, 8-ball commit c596c13fb431ac7c8f7aa003181de85c856195d6, [run](https://github.com/Tanguille/8-ball/actions/runs/44162941364).

| Method | Decided | Wrong | Accuracy of decided | Correct of all | Correct vs d3 (95% CI) | Accuracy at 90% coverage | Mean time | Speed vs d3 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| d3 (vllm-d3-nano-Q8_0.gguf) | 90% | 10 | 98.2% | 88.8% | reference | 98.3% | 881 ms | 1.0x |
| rerank (bge-reranker-v2-m3-Q8_0.gguf) | 0% | 0 | - | 0.0% | -88.8 (-91.3 to -86.3) | 55.4% | 134 ms | 6.6x |
| chat (Qwen3.5-0.8B-Q8_0.gguf) | 35% | 31 | 85.2% | 29.7% | -59.2 (-63.7 to -54.3) | 56.3% | 506 ms | 1.7x |

Per task, accuracy of decided (decided share):

| Method | spam | positive | lost-card |
| --- | --- | --- | --- |
| d3 (vllm-d3-nano-Q8_0.gguf) | 100.0% (98%) | 93.8% (81%) | 100.0% (92%) |
| rerank (bge-reranker-v2-m3-Q8_0.gguf) | - (0%) | - (0%) | - (0%) |
| chat (Qwen3.5-0.8B-Q8_0.gguf) | 90.0% (10%) | 76.1% (56%) | 97.4% (38%) |

600 cases per method (yes/no: spam 100/100, positive 100/100, lost-card 100/100), 0 errors. Hardware: Intel(R) Xeon(R) 6973P-C, no GPU, CPU only.

Compared with the baseline (decided, wrong):

| Method | Task | Decided | Wrong | Result |
| --- | --- | --- | --- | --- |
| d3 | spam | 196 (+0) | 0 (+0) | same |
| d3 | positive | 162 (+0) | 10 (+0) | same |
| d3 | lost-card | 185 (+0) | 0 (+0) | same |
| rerank | spam | 0 (+0) | 0 (+0) | same |
| rerank | positive | 0 (+0) | 0 (+0) | same |
| rerank | lost-card | 0 (+0) | 0 (+0) | same |
| chat | spam | 20 (+2) | 2 (+1) | same |
| chat | positive | 113 (-3) | 27 (-2) | worse (report only) |
| chat | lost-card | 76 (-5) | 2 (+0) | worse (report only) |
