---
title: Schedules
description: Wake a teammate once, on chosen days at a time, or on an interval, from its pane or from the teammate itself.
---

A schedule is a message delivered to a teammate at a time. It arrives the
same way as something you type: the teammate starts if it was idle, reads
the prompt, and works.

## Add one yourself

Open the teammate's pane (<kbd>⌘</kbd>/<kbd>Ctrl</kbd> + <kbd>I</kbd>) and,
under **Schedules**, choose **Add a schedule**:

- **Once**: a day and a time (**At**), such as `9:30`, `14:00` or `2pm`.
- **On days**: the **Days** it runs (weekdays to start with; at least one)
  and the time (**At**), on this computer's clock. The line under the time
  reads it back, such as "Weekdays at 9:00 AM".
- **Every**: so many minutes, hours or days, starting from now.
- **Say**: what to ask when it fires.
- **Quiet**: the run's words land in the conversation as thoughts rather than
  as a reply that lights up the team list.

When a quiet run finds something you need to see, a Hotline Agent teammate
hands it over, and once the run is done it tells you in the conversation,
like any other reply, and your phone is notified. A run that finds nothing
says nothing. Coding tools such as Claude Code have no way to speak up from a
quiet run, so their quiet runs stay quiet.

A run that asks a colleague hears the reply in the run. A **Once** schedule,
or a repeating one's last run, waits for that reply before it ends: its row
says "Waiting on Huck" until the reply comes, for at most three days. Stop or
End job in its pane ends it at once.

Once a teammate has schedules, its conversation's header shows how many and
when the next one fires; press it to open the list. Cancel a schedule from
the list.

Bounds:

| | |
| --- | --- |
| Once | 1 second to 30 days ahead |
| On days | any days of the week, at a time of day |
| Every | 15 seconds to 7 days (from the pane: whole minutes, hours or days) |
| Prompt | up to 8000 bytes (fewer characters in scripts outside basic Latin) |
| Jobs per teammate | 20 |

A schedule you add runs whatever the teammate's **Background work** setting.

## Let the teammate schedule itself

Turn on **Background work**, under **Permissions** in the teammate's pane.
The teammate can then use `schedule`, `loop`, `list_schedules`,
`cancel_schedule` and `update_schedule` (which turns **Quiet** on or off
for one of its jobs, as the checkbox does), and can say "check the build again in 20 minutes" and
mean it. Its jobs show in the same list. Turning **Background work** off
pauses the jobs it made without deleting them; turning it on wakes them.

## While Hotline is closed

Nothing fires while Hotline is not running. A schedule that came due while
Hotline was closed fires once when it opens again, not once per missed
interval. A repeating schedule whose time passed more than two hours before
Hotline could run it skips that run and waits for its next time; its row
says "Skipped" with the day and time it was due until it next runs. Closing the window does not close Hotline; quitting from the tray
or the menu does. A desk on a [server](/docs/server/) runs around the clock.
