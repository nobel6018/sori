# 화자 임베딩 모델 비교 (2026-11-02)

sori는 화자 분리와 목소리 기억에 3D-Speaker ERes2Net base(`3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx`)를 씁니다. 중국어로 학습한 모델이라 한국어, 영어가 섞인 회의에 맞는지 sherpa-onnx가 내려받기로 제공하는 모델 9개와 비교했습니다. 결론은 지금 모델을 유지합니다.

## 1. 한국어 화자 구분 (Zeroth Korean test)

Zeroth Korean test 묶음(10명, 457문장, 한국어 낭독)으로 모든 문장 쌍의 유사도를 쟀습니다. 회의 구간은 짧으므로 문장 가운데 2초만 자른 경우도 쟀습니다. EER은 같은 사람을 다르다고 보는 비율과 다른 사람을 같다고 보는 비율이 같아지는 지점의 오류율이며, 낮을수록 좋습니다.

| 모델 | 학습 데이터 | 크기 | EER 전체 | EER 2초 | 같은 사람과 다른 사람 차이(2초) |
|---|---|---|---|---|---|
| ERes2Net base (지금) | 3D-Speaker 중국어 | 37MB | 0.3% | 1.7% | 0.46 |
| ERes2Net large | 3D-Speaker 중국어 | 110MB | 0.3% | 1.4% | 0.49 |
| ERes2NetV2 | 중국어 20만 명 | 68MB | 0.3% | 1.3% | 0.44 |
| ERes2Net base 200k | 중국어 20만 명 | 37MB | 0.3% | 1.3% | 0.46 |
| CAM++ zh_en advanced | 중국어, 영어 | 26MB | 0.3% | 1.4% | 0.42 |
| ERes2Net VoxCeleb | VoxCeleb 영어 | 25MB | 0.2% | 2.6% | 0.43 |
| NeMo TitaNet large | 영어 | 96MB | 0.4% | 2.7% | 0.44 |
| WeSpeaker ResNet34 LM | VoxCeleb 영어 | 25MB | 6.8% | 14.2% | 0.26 |
| WeSpeaker ResNet293 LM | VoxCeleb 영어 | 109MB | 7.7% | 15.4% | 0.24 |

3D-Speaker 계열은 서로 차이가 0.4%p 안쪽입니다. 10명짜리 묶음에서 이 정도는 우연과 구분되지 않습니다. WeSpeaker ResNet은 다른 사람끼리도 유사도가 0.5~0.6으로 높아 오류가 5~10배입니다. 영어 데이터로 학습해 한국어 목소리를 덜 구분하는 것으로 보입니다.

## 2. 실제 회의 화자 수 (녹음 4개)

화자 수를 아는 회의 녹음 4개(3, 5, 4, 4명)로 기준값별 추정치를 쟀습니다(`cargo run --release --example speaker_count_check -- <모델> <녹음 폴더>=<화자 수> ...`). 모델마다 맞는 기준값이 달라 0.10~0.80을 다 봤고, 모델별로 가장 잘 맞은 기준값만 적습니다.

| 모델 | 가장 좋은 기준값 | 추정 (정답 3, 5, 4, 4) | 맞은 개수 | 틀린 명수 합 |
|---|---|---|---|---|
| ERes2Net base (지금, 0.45) | 0.45 | 3, 5, 6, 4 | 3 | 2 |
| ERes2NetV2 | 0.50 | 3, 5, 5, 4 | 3 | 1 |
| CAM++ zh_en advanced | 0.35 | 3, 4, 4, 4 | 3 | 1 |
| ERes2Net large | 0.25 | 2, 4, 4, 3 | 1 | 3 |
| ERes2Net base 200k | 0.45 | 2, 4, 4, 3 | 1 | 3 |
| NeMo TitaNet large | 0.40 | 2, 4, 7, 4 | 1 | 5 |
| ERes2Net VoxCeleb | 0.35 | 2, 4, 7, 4 | 1 | 5 |
| WeSpeaker ResNet34 LM | 0.75 | 4, 6, 2, 2 | 0 | 6 |
| WeSpeaker ResNet293 LM | 0.80 | 3, 4, 2, 2 | 1 | 5 |

ERes2NetV2와 CAM++가 지금 모델보다 한 명 덜 틀렸지만 녹음 4개에서 한 명 차이라 우연일 수 있습니다. 세 모델 모두 기준값을 0.05만 옮겨도 결과가 크게 바뀝니다. 남은 오류는 대부분 32분 회의(현장을 돌며 녹음)에서 나왔고, 어느 모델도 이 녹음을 꾸준히 맞히지 못했습니다. 모델보다 녹음 환경(울림, 거리)의 영향으로 봅니다. WeSpeaker ResNet은 기준값을 낮추면 모두 한 명으로 합칩니다.

## 결론

- 지금 모델(ERes2Net base, 기준값 0.45)을 유지합니다. 더 나은 후보가 있어도 차이가 측정 오차 안쪽입니다
- 모델을 바꾸면 저장한 목소리 특징(`voices.json`, 전사 파일의 `speakerEmbeddings`)을 모두 다시 만들어야 합니다. 이 비용을 치를 만한 차이가 아닙니다
- 회의 녹음이 10개 넘게 쌓이면 이 비교를 다시 합니다. 그때도 ERes2NetV2나 CAM++가 앞서면 교체를 검토합니다
- 화자 수를 알면 직접 적는 것이 어느 모델보다 정확합니다

## 다시 재는 방법

- 회의: `src-tauri/examples/speaker_count_check.rs`
- 한국어 묶음: Hugging Face `kresnik/zeroth_korean`의 `data/test-00000-of-00001.parquet`(60MB)을 받아, 문장마다 `sherpa_onnx.SpeakerEmbeddingExtractor`로 임베딩을 뽑고 모든 쌍의 코사인 유사도로 EER을 계산합니다
- 모델 파일: sherpa-onnx Release `speaker-recongition-models`
