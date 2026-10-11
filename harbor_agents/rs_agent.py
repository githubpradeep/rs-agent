"""Harbor installed-agent plugin for rs-agent on Terminal-Bench."""

import os
import shlex
from pathlib import Path

from harbor.agents.installed.base import BaseInstalledAgent, with_prompt_template
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext

# linux/amd64 release produced by `./scripts/tb-daytona.sh --build`
BINARY = (
    Path(__file__).resolve().parents[1]
    / "target"
    / "x86_64-unknown-linux-gnu"
    / "release"
    / "rs-agent"
)
DEFAULT_MODEL = "deepseek/deepseek-v4.1-flash"


class RsAgent(BaseInstalledAgent):
    @staticmethod
    def name() -> str:
        return "rs-agent"

    def version(self) -> str | None:
        return "0.1.0"

    def _model(self) -> str:
        raw = (self.model_name or DEFAULT_MODEL).strip()
        prefix = "openrouter/"
        if raw.startswith(prefix):
            raw = raw[len(prefix) :]
        return raw or DEFAULT_MODEL

    async def install(self, environment: BaseEnvironment) -> None:
        if not BINARY.is_file():
            raise RuntimeError(
                f"Linux binary not found at {BINARY}. Run ./scripts/tb-daytona.sh --build"
            )
        await self.exec_as_root(
            environment,
            "if command -v apt-get >/dev/null 2>&1; then "
            "apt-get update && DEBIAN_FRONTEND=noninteractive "
            "apt-get install -y libssl3 ca-certificates python3; "
            "else echo 'rs-agent needs a Debian/Ubuntu image with apt-get' >&2; exit 1; fi",
            timeout_sec=300,
        )
        await self.exec_as_root(environment, "mkdir -p /usr/local/bin")
        await environment.upload_file(BINARY, "/usr/local/bin/rs-agent")
        await self.exec_as_root(environment, "chmod 755 /usr/local/bin/rs-agent")

    @with_prompt_template
    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        key = os.environ.get("OPENROUTER_API_KEY", "")
        if not key:
            raise RuntimeError("OPENROUTER_API_KEY is not set in the Harbor process")
        model = self._model()
        # --thinking-budget 0 skips OpenAI reasoning_effort, which OpenRouter
        # rejects for this DeepSeek route. The model still thinks on its side.
        command = (
            "rs-agent -a --no-context-files --timeout 7200 --thinking-budget 0 "
            "--provider openrouter "
            f"--model {shlex.quote(model)} "
            f"-p {shlex.quote(instruction)}"
        )
        await self.exec_as_agent(
            environment,
            command,
            env={"OPENROUTER_API_KEY": key},
            timeout_sec=7200,
        )
