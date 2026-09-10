## Most important information
- It is an API for 'autocalls.ai' to perform actions and fetch information during calls
- It is built with Rust, full set of features and dependencies in root Cargo.toml
- Uses docker-compose.yml
- To save code changes, update.sh script should be run in order for docker container to get recreated

## Architectural information
- PostgreSql for database, Redis for cache and session.
- Routes, Middlewares at app/src/routes/api.rs and app/src/routes/middleware.rs
- Tests at app/src/routes/lib.rs
- Migrations at migrations/
- Config at app/config/
- 'tracing' for logs with appropriate macros

## Requirements for code updates
- Adhere to all listed instructions below when making any task in this project, no exceptions
- If something is already implemented some type of way, similar additions must follow the way
- If code addition created code duplication, the code must be generalized for further extensions using design patterns. When design pattern is implemented in a rust file, top of the file must contain comments describing the pattern.
- Code must be properly commented, in a human readable form using best practices for comments & readability.
- Always do 'cargo fmt' after code addition
- Always write test to prove the code works
- Always run tests before finishing a task to update code
- Never guess when reasoning about code architecture, verify every assumption
- All the additional
- When temporary files are needed for work use .claude/tmp
- When repetitive work needs a tool/script, add them in .claude/tools and document each tool in tools/README.md
- Always check if tool which helps with a task exists in .claude/tools/README.md
- Any notes about project that might be helpful for future sessions must be documented in .claude/CLAUDENOTES.md in a fast to read form