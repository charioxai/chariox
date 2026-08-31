# Chariox runtime

Chariox coordinates people and agents inside durable collaborative runtimes. This glossary names the product concepts that remain the same across local, remote, Web, and native clients.

## Collaboration

**Room**:
A durable collaboration runtime containing users, agents, workflows, history, and at most one default shared Environment. A Room may remain active when no client is attached.
_Avoid_: Workspace, Session, Chat

**Workspace**:
The project files and source-control context bonded to a Room or agent. A Workspace is not the Room and does not own live agents, browser state, or interaction history.
_Avoid_: Room, Session

**Attachment**:
One client connection participating in a Room. Attachments observe and request changes but never become the authority for Room state.
_Avoid_: Owner, Controller

## Shared computer

**Environment**:
The Room-owned browser and graphical computer shared by its users and agents. Browser and Computer are two ways to observe and act in the same Environment.
_Avoid_: Agent browser, Viewer, Desktop session

**Browser mode**:
The structured projection of an Environment, including tabs, page observations, element references, navigation, and browser actions.
_Avoid_: Browser instance

**Computer mode**:
The graphical projection of an Environment, including the full display, desktop programs, screenshots, pointer input, keyboard input, and clipboard actions.
_Avoid_: Separate desktop

**Tab**:
A Room-visible browser page with an identity that remains stable while its underlying browser target is recoverable. Navigation changes the Tab's document revision, not its identity.
_Avoid_: CDP target, Page handle

**Actor**:
A user or agent that observes or acts in an Environment. Presence alone grants no input ownership.
_Avoid_: Client, Connection

**Action**:
One attributed attempt by an Actor to observe or change a Tab or the desktop. An Action has a target, lifecycle, and recorded outcome.
_Avoid_: Tool call, Input event

**Input target**:
The mutation scope reserved by an Action. Page mutations usually target one Tab; tab lifecycle and graphical Actions may target the desktop and affected Tabs.
_Avoid_: Lock

**Canonical viewport**:
The single display dimensions and scale shared by the browser, streamer, screenshots, coordinates, and every viewer.
_Avoid_: Client size, Local viewport

**Takeover**:
An explicit transfer of an Input target to a user. Takeover cancels or pauses the active agent Action on that target and never reverts ownership silently.
_Avoid_: Focus, Hover
