# TODO

このファイルは、`proj-finder` の機能開発状況を
言語別対応とは切り離して管理するためのチェックリスト。

言語ごとの AST 対応はここには含めず、
Agentic Coding Tool から見た使い勝手と
探索基盤そのものの進捗だけを扱う。

## コア走査とインデックス生成

- [x] リポジトリルートを `.git` から解決する
- [x] `.gitignore` に従って走査対象を絞る
- [x] ファイル列挙を行う
- [x] 位置要約 (`location_summary`) を生成する
- [x] 内容要約 (`content_summary`) を生成する
- [x] タグ (`tags`) を生成する
- [x] タグに `source` / `confidence` / `evidence` を持たせる
- [x] Git 差分 (`git diff`, `git ls-files`) を使って変更候補を高速に絞る
- [x] 差分更新で再解析対象だけを更新する
- [x] Git 差分が使えない場合の hash fallback を実装する
- [x] インデックスの永続化形式を導入する
- [x] フルスキャンと差分更新を切り替える仕組みを持つ

## 検索機能

- [x] 構造化クエリ (`must` / `any` / `exclude` / `prefer` / `limit`) を受け取る
- [x] タグ一致に基づくスコアリングを行う
- [x] ヒット理由 (`matched_must` / `matched_any` / `matched_prefer`) を返す
- [x] ヒット理由の詳細 (`source` / `confidence` / `evidence`) を返す
- [x] 位置タグと内容タグの両方で検索できる
- [x] 本文 `grep` 条件で行マッチを返せる
- [x] `grep` の人間向け整形出力とハイライトを追加する
- [x] `--incremental` 時の grep-only 検索を manifest / shard lazy load で処理する
- [x] `search` の人間向け整形出力を追加する
- [x] 検索結果の出力量を細かく制御できるようにする
- [x] 検索結果から追加探索しやすい補助出力を追加する

## CLI

- [x] `clap` ベースの CLI を導入する
- [x] `scan` サブコマンドを提供する
- [x] `search` サブコマンドを提供する
- [x] `search` で人間向けのフラグ指定 (`--must` / `--any` / `--exclude` / `--prefer` / `--grep`) を提供する
- [x] `grep` サブコマンドを提供する
- [x] 旧来の `cargo run -- .` を互換動作として残す
- [x] 出力形式の切り替えオプションを追加する
- [ ] Agent プロファイル切り替え (`--profile ...`) を追加する
- [x] 差分更新用サブコマンドまたはフラグを追加する
- [x] Git 差分ベース更新とフルスキャンを選べるようにする

## Agent 最適化

- [ ] Codex 向けプロファイルを定義する
- [ ] Codex 向け探索戦略を定義する
- [ ] Codex 向け出力フォーマットを定義する
- [ ] Claude Code 向けプロファイルを定義する
- [ ] Claude Code 向け探索戦略を定義する
- [ ] Claude Code 向け出力フォーマットを定義する
- [ ] Agent ごとの既定値を CLI に反映する

## フレームワーク対応

- [ ] フレームワーク対応の親方針を整理する
- [ ] Next.js 規約ベースの分類と role 推定を追加する
- [ ] React 規約ベースの分類と role 推定を追加する
- [ ] Loco 規約ベースの分類と role 推定を追加する
- [ ] 言語対応とフレームワーク対応の責務分離を README に明記する

## 品質管理

- [x] ユニットテストを整備する
- [x] `cargo fmt --check` を CI で実行する
- [x] `cargo clippy -D warnings` を CI で実行する
- [x] `cargo test` を CI で実行する
- [x] チェック通過後に Release する GitHub Actions を追加する
- [ ] 初回の tag push による Release 動作を実地確認する

## ドキュメント

- [x] README に全体方針を整理する
- [x] README に CLI の使い方を記載する
- [x] README に CI / Release の流れを記載する
- [ ] README に Agent プロファイル設計を追記する
- [x] README に差分更新の仕様を追記する
- [x] README に Git 差分 fast path と hash fallback の役割分担を追記する

## 運用メモ

- [ ] このチェックリストを実装に合わせて継続更新する
- [ ] Issue と TODO の対応関係を必要に応じて整理する
