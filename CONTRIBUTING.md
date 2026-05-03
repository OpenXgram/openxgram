# OpenXgram 기여 가이드 / Contributing Guide

OpenXgram에 기여해 주셔서 감사합니다. 이 문서는 버그 신고, 기능 제안, 코드 기여 방법을 안내합니다.

---

## 버그 신고 / Bug Reports

GitHub Issues를 사용합니다. 신고 시 다음을 포함해 주세요:

- 운영체제 및 Rust 버전 (`rustc --version`)
- 재현 단계 (step-by-step)
- 예상 동작 vs 실제 동작
- 에러 메시지 또는 로그 전문

보안 취약점은 Issues 대신 `security@openxgram.org` (placeholder)로 비공개 신고해 주세요.
자세한 내용은 [SECURITY.md](./SECURITY.md)를 참조하세요.

---

## 기능 제안 / Feature Requests

GitHub Issues에 `enhancement` 라벨로 등록해 주세요.
PRD나 설계 문서가 있으면 함께 첨부하면 검토가 빠릅니다.

처음 기여하신다면 `good-first-issue` 라벨이 붙은 이슈를 찾아보세요.

---

## PR 흐름 / Pull Request Flow

```bash
# 1. 저장소 포크 후 클론
git clone https://github.com/YOUR_USERNAME/openxgram.git
cd openxgram

# 2. 기능 브랜치 생성 (main 기반)
git checkout -b feat/your-feature-name

# 3. 작업 후 커밋 (Conventional Commits 형식)
git commit -s -m "feat(keystore): add HD key derivation"

# 4. 포크에 푸시
git push origin feat/your-feature-name

# 5. GitHub에서 main 대상 PR 생성
```

PR 설명에는 변경 이유와 테스트 방법을 적어 주세요.

---

## 커밋 메시지 형식 / Commit Message Convention

[Conventional Commits](https://www.conventionalcommits.org/) 표준을 따릅니다:

```
<type>(<scope>): <description>

[optional body]

[optional footer]
Signed-off-by: Your Name <your@email.com>
```

타입: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`, `perf`, `ci`

예시:
- `feat(cli): add xgram status command`
- `fix(db): handle WAL mode initialization error`
- `docs: update README with data directory path`

---

## 코딩 스타일 / Coding Style

Rust 코드는 다음 도구로 자동 포맷 및 검사합니다:

```bash
# 포맷 적용
cargo fmt --all

# 린트 검사 (경고 0개 유지)
cargo clippy --all-targets --all-features -- -D warnings

# 테스트 실행
cargo test --workspace
```

PR 병합 전 CI에서 위 세 명령이 모두 통과해야 합니다.

---

## DCO Sign-off

모든 커밋에 DCO(Developer Certificate of Origin) sign-off가 필요합니다:

```bash
git commit -s -m "your message"
```

`-s` 플래그가 자동으로 `Signed-off-by: Name <email>` 라인을 추가합니다.

---

## 라이선스

기여한 코드는 [MIT License](./LICENSE)에 따라 배포됩니다.
