# Benchmark: matrix

d3 ran on the CPU of the GitHub runner. The other models ran at a hosted provider, so their times include the network and the rate limit. Compare accuracy between rows, not time.

Run on 2026-11-22, 8-ball commit 42613ece1d6111b20039c3f095f43f98334ccd6d, [run](https://github.com/Tanguille/8-ball/actions/runs/45018640657).

Left off the published page, with errors on more than 1% of their cases: hosted-qwen-3-8-27b, hosted-gemma-4-26b, d3, then hosted-qwen-3-8-27b for unsure.

| Method | Decided | Wrong | Accuracy of decided | Correct of all | Correct vs d3 (95% CI) | Accuracy at 90% coverage | Mean time | Speed vs d3 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| d3 (vllm-d3-nano-Q8_0.gguf) | 90% | 5 | 98.1% | 88.0% | reference | 98.1% | 1190 ms | 1.0x |
| hosted-clef (clef) | 91% | 3 | 98.9% | 90.0% | +2.0 (-1.7 to +6.0) | 98.9% | 3795 ms | 0.3x |
| hosted-clef-flash (clef-flash) | 92% | 8 | 97.1% | 89.3% | +1.3 (-3.0 to +5.7) | 97.8% | 3717 ms | 0.3x |

Per task, accuracy of decided (decided share):

| Method | spam | positive | lost-card |
| --- | --- | --- | --- |
| d3 (vllm-d3-nano-Q8_0.gguf) | 100.0% (97%) | 93.7% (79%) | 100.0% (93%) |
| hosted-clef (clef) | 98.9% (93%) | 98.8% (82%) | 99.0% (98%) |
| hosted-clef-flash (clef-flash) | 93.5% (92%) | 98.8% (86%) | 99.0% (98%) |

300 cases per method (yes/no: spam 50/50, positive 50/50, lost-card 50/50), 3 errors. Hardware: AMD EPYC 9V45 96-Core Processor, no GPU, CPU only.
