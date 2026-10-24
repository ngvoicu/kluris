# Testing Knowledge Base

Language-agnostic testing reference for the specmint-tdd research subagent and
forge workflows. Use this to identify the right tools, patterns, and
anti-patterns for any project stack.

## Test Frameworks by Language

| Language | Framework | Runner | Config File |
|----------|-----------|--------|-------------|
| TypeScript/JS | Vitest | Vite | `vitest.config.ts` |
| TypeScript/JS | Jest | Jest CLI | `jest.config.ts` / `jest.config.js` |
| TypeScript/JS | Mocha | Mocha CLI | `.mocharc.yml` |
| TypeScript/JS | Node.js Test Runner | `node --test` | None (built-in) |
| Python | pytest | pytest CLI | `pyproject.toml` / `pytest.ini` / `conftest.py` |
| Python | unittest | `python -m unittest` | None (built-in) |
| Java | JUnit 5 | Maven Surefire / Gradle | `pom.xml` / `build.gradle` |
| Java | TestNG | Maven Surefire / Gradle | `testng.xml` |
| Kotlin | JUnit 5 | Gradle | `build.gradle.kts` |
| Kotlin | Kotest | Gradle | `build.gradle.kts` |
| Go | testing (stdlib) | `go test` | None (built-in) |
| Rust | built-in (`#[test]`) | `cargo test` | `Cargo.toml` |
| C# | xUnit | `dotnet test` | `.csproj` |
| C# | NUnit | `dotnet test` | `.csproj` |
| C# | MSTest | `dotnet test` | `.csproj` |

## Mocking Libraries by Language

| Language | Library | Style | Notes |
|----------|---------|-------|-------|
| TypeScript/JS | MSW | Network intercept | Mock HTTP at the network level; ideal for API consumers |
| TypeScript/JS | `vi.mock` / `jest.mock` | Module replacement | Built into Vitest / Jest; replaces entire modules |
| TypeScript/JS | Sinon.JS | Spy/stub/mock | Standalone; works with any runner |
| Python | `pytest-mock` (mocker) | Patch/spy | Wrapper around `unittest.mock`; pytest-native |
| Python | `unittest.mock` | Patch/spy | Stdlib; `patch`, `MagicMock`, `AsyncMock` |
| Java | Mockito | Proxy-based mock | De facto standard for Java unit tests |
| Java | WireMock | HTTP server mock | Mock external HTTP services |
| Kotlin | MockK | Coroutine-aware mock | Kotlin-idiomatic; supports suspend functions |
| Go | `testify/mock` | Interface mock | Part of the testify suite |
| Go | `gomock` | Code-gen mock | Google's official mocking framework |
| Rust | `mockall` | Trait mock | Proc-macro based; mocks any trait |
| C# | Moq | Proxy-based mock | LINQ-style setup; most popular in .NET |
| C# | NSubstitute | Proxy-based mock | Simpler syntax alternative to Moq |

## Testcontainers by Language

| Language | Package | Key Modules |
|----------|---------|-------------|
| TypeScript/JS | `testcontainers` | `GenericContainer`, `PostgreSqlContainer`, `RedisContainer`, `KafkaContainer` |
| Python | `testcontainers` | `PostgresContainer`, `RedisContainer`, `MySqlContainer`, `KafkaContainer` |
| Java | `org.testcontainers:testcontainers` | `PostgreSQLContainer`, `MySQLContainer`, `KafkaContainer`, `LocalStackContainer` |
| Kotlin | `org.testcontainers:testcontainers` | Same as Java; Kotest extension: `io.kotest.extensions:kotest-extensions-testcontainers` |
| Go | `github.com/testcontainers/testcontainers-go` | `postgres`, `redis`, `kafka`, `localstack` modules |
| Rust | `testcontainers` | `GenericImage`, `images::postgres`, `images::redis` |
| C# | `Testcontainers` | `PostgreSqlBuilder`, `MsSqlBuilder`, `RedisBuilder`, `KafkaBuilder` |

## Isolation Patterns

| Pattern | Isolation Approach | Speed | Fidelity | When to Use |
|---------|-------------------|-------|----------|-------------|
| Unit (pure logic) | No deps; pure functions | Fastest (<1ms) | Low | Business rules, calculations, transformations |
| Unit (with deps) | Mock/stub collaborators | Fast (~1-10ms) | Medium | Service layer, handlers with injected deps |
| Integration (DB) | Testcontainers or test DB | Medium (~100ms-1s) | High | Repository layer, migrations, queries |
| Integration (HTTP) | MSW / WireMock / VCR | Medium (~10-100ms) | High | API clients, webhook handlers |
| Contract | Pact / Schema validation | Medium (~10-100ms) | High | Service boundaries, API contracts |
| E2E (backend) | Testcontainers + mocked externals | Medium (~100ms-2s) | Highest | Full API request lifecycle: HTTP in → middleware → handler → service → DB → response out |
| E2E (frontend) | Playwright / Cypress | Slow (~1-30s) | Highest | Browser-based user flows, critical paths |
| E2E (full stack) | Testcontainers + browser runner | Slow (~2-30s) | Highest | Complete user journey: UI → API → DB → response → UI |

## E2E Testing by Language

E2E tests exercise the full request/response lifecycle in isolation — no external
network calls. Use testcontainers for real databases and services, mocks for
third-party APIs. Backend e2e tests don't need a browser.

| Language | HTTP Testing | DB Isolation | External API Mocking | Browser E2E |
|----------|-------------|-------------|---------------------|------------|
| TypeScript/JS | Supertest, undici | Testcontainers (PostgreSqlContainer, RedisContainer) | MSW (setupServer) | Playwright, Cypress |
| Python | httpx (AsyncClient), TestClient (FastAPI/Starlette) | Testcontainers (PostgresContainer) | responses, respx, VCR.py | Playwright |
| Java | MockMvc, RestAssured, WebTestClient | Testcontainers (PostgreSQLContainer) | WireMock | Selenium, Playwright |
| Kotlin | MockMvc, WebTestClient, Ktor testApplication | Testcontainers (PostgreSQLContainer) | WireMock, MockK | Playwright |
| Go | net/http/httptest | Testcontainers (postgres module) | httptest (custom handler) | Rod, chromedp |
| Rust | actix_web::test, axum::test | Testcontainers (images::postgres) | wiremock (MockServer) | — |
| C# | WebApplicationFactory, HttpClient | Testcontainers (PostgreSqlBuilder) | WireMock.Net | Playwright |

### Backend E2E Pattern

The goal is to test the full request lifecycle without leaving the process:

```
HTTP request (Supertest/httpx/MockMvc/httptest)
  → Middleware stack (auth, rate limiting, validation)
    → Route handler / Controller
      → Service / Use case layer
        → Repository → Real DB (Testcontainers)
        → HTTP client → Mocked external API (MSW/WireMock)
      ← Response built
    ← Middleware post-processing
  ← HTTP response asserted
```

Key principles:
- **Real database**: Testcontainers with the production DB engine, never in-memory substitutes
- **Mocked externals**: Third-party APIs (OAuth providers, payment gateways, email services) mocked at the HTTP boundary
- **No network calls**: Everything runs in-process or in local containers
- **Full middleware stack**: Auth, validation, error handling all exercised
- **Realistic data**: Use factories/fixtures, not minimal stubs

## Anti-Patterns

Avoid these patterns in test suites. They create fragile, slow, or misleading tests.

### 1. In-Memory DB Substitutes

Using SQLite as a stand-in for PostgreSQL or H2 for MySQL. Different engines
have different SQL dialects, constraint behaviors, and concurrency semantics.
Tests pass locally, fail in production.

**Instead**: Use Testcontainers with the real database engine.

### 2. Mocking Internals

Mocking private methods, internal modules, or implementation details. Tests
become tightly coupled to the implementation and break on every refactor.

**Instead**: Mock at architectural boundaries (ports/adapters). Test behavior,
not implementation.

### 3. Execution Order Dependencies

Tests that rely on other tests running first (shared state, database rows
created by earlier tests). Leads to flaky failures when tests run in parallel
or in different order.

**Instead**: Each test sets up its own state and tears it down. Use
`beforeEach` / setup fixtures.

### 4. Sleep-Based Synchronization

Using `sleep(2000)` or `time.sleep(3)` to wait for async operations.
Slow, flaky, and race-condition-prone.

**Instead**: Use polling with assertions (`waitFor`, `eventually`,
`assertj-await`), event-based synchronization, or test-specific hooks.

### 5. Snapshot Tests for Business Logic

Using snapshot tests (Jest snapshots, approval tests) to verify business logic
output. Snapshots are useful for UI regression; for logic they create
"accept whatever the code does" tests that miss actual bugs.

**Instead**: Write explicit assertions for expected values. Use
property-based tests for complex outputs.

### 6. Testing Frameworks Instead of Application Code

Writing tests that verify framework behavior ("does Express route to my
handler?") rather than application logic. Wastes time testing code you don't
own.

**Instead**: Test your handler logic directly. Trust the framework.

### 7. Overmocking

Mocking so many collaborators that the test verifies nothing except the mock
wiring. The test becomes a mirror of the implementation.

**Instead**: Prefer integration tests with real collaborators where practical.
Reserve mocking for external boundaries.

### 8. Ignoring Test Failures

Skipping or commenting out failing tests instead of fixing them. Erodes
trust in the suite.

**Instead**: Fix the test or remove it. A skipped test is invisible debt.

## Coverage Tools by Language

| Language | Tool | Report Formats | Notes |
|----------|------|----------------|-------|
| TypeScript/JS | `v8` (Vitest built-in) | text, lcov, html, json | Native V8 coverage; fast |
| TypeScript/JS | `istanbul` / `nyc` | text, lcov, html, json | Instrumenting coverage; works with any runner |
| TypeScript/JS | `c8` | text, lcov, html, json | V8-based; standalone CLI |
| Python | `coverage.py` | text, html, xml, json, lcov | `pytest-cov` integrates with pytest |
| Java | JaCoCo | html, xml, csv | Gradle/Maven plugin; standard for Java |
| Kotlin | JaCoCo / Kover | html, xml | Kover is JetBrains' Kotlin-native alternative |
| Go | `go test -cover` | text, html | Built-in; `-coverprofile` for file output |
| Rust | `cargo-tarpaulin` | text, html, xml, json, lcov | Most popular; also `cargo-llvm-cov` for LLVM-based |
| C# | `coverlet` | text, lcov, cobertura, json | Cross-platform; integrates with `dotnet test` |

## Mutation Testing Tools by Language

| Language | Tool | Mutators | Integration |
|----------|------|----------|-------------|
| TypeScript/JS | Stryker | Arithmetic, conditional, string, block, array | Vitest, Jest, Mocha, Karma plugins |
| Python | `mutmut` | Arithmetic, comparison, keyword, string | pytest; `mutmut run` CLI |
| Python | `cosmic-ray` | Broad mutator set | pytest; distributed execution support |
| Java | Pitest (PIT) | Conditionals, math, returns, void, increments | Maven/Gradle plugin; JUnit 4/5 |
| Kotlin | Pitest + `pitest-kotlin` | Same as Pitest + Kotlin-specific | Gradle plugin with Kotlin support |
| Go | `go-mutesting` | Conditionals, branches, expressions | `go test` integration |
| Rust | `cargo-mutants` | Replace functions with default returns | Cargo plugin; `cargo mutants` CLI |
| C# | Stryker.NET | Arithmetic, conditional, string, LINQ, regex | `dotnet-stryker` CLI; xUnit/NUnit/MSTest |

## Property-Based Testing Tools by Language

| Language | Tool | Style | Notes |
|----------|------|-------|-------|
| TypeScript/JS | fast-check | Shrinking, arbitrary generators | Works with any runner; `fc.assert(fc.property(...))` |
| Python | Hypothesis | Shrinking, strategies, stateful testing | pytest integration; `@given(...)` decorator |
| Java | jqwik | JUnit Platform engine, shrinking | `@Property`, `@ForAll` annotations |
| Kotlin | Kotest property testing | Shrinking, arb generators | Built into Kotest: `forAll(Arb.int()) { ... }` |
| Go | `testing/quick` (stdlib) | Basic; limited shrinking | Built-in; `quick.Check` for simple properties |
| Go | `gopter` | Shrinking, generators | More capable alternative to stdlib |
| Rust | `proptest` | Shrinking, strategy-based | `proptest! { ... }` macro; strategies compose |
| Rust | `quickcheck` | Shrinking, `Arbitrary` trait | Inspired by Haskell QuickCheck |
| C# | FsCheck | Shrinking, generators | Works with xUnit/NUnit; `Prop.ForAll(...)` |
