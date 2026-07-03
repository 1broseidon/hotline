# Templates

Templates are starting points for a hotline setup. Each one is a folder you can copy into a project: instructions for the agent, a filing structure, a voice. They capture patterns that work in real daily use, generalized so you can adopt them and then reshape them by chatting with your own agent.

To use one today, copy the template folder's contents into the project you want to text with (or make the folder itself the project), then run `hotline init` there and `hotline start` as usual. The agent picks up the `CLAUDE.md` and `HOTLINE.md` on launch and starts working the system from the first message. Mission-control also ships as a plugin: `claude plugin marketplace add 1broseidon/hotline`, then `claude plugin install mission-control@hotline`, then `/mission-control:init` in the target folder.

There is one template for now: [mission-control](mission-control/), our take on what makes a good texting agent. Built your own setup around hotline? Open a PR with your template. The interesting ones come from real daily use.
