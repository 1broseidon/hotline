---
title: Schedules
description: Wake a teammate once, or on an interval, from the pane or from the teammate itself.
---

A schedule is a message delivered to a teammate at a time. It fires through
the same door as you typing: the teammate starts if it was idle, reads the
prompt, and works.

## Add one yourself

Open the teammate's conversation and choose **Schedules** in its band. A
schedule is either **once**, at a date and time, or **every** so many
seconds, minutes, hours or days. The prompt is what to ask when it fires.
Each job has a **quiet** switch: quiet runs land in the tape as thoughts
rather than as a reply that lights up the rail. When a quiet run finds
something you need to see, a Hotline Agent teammate hands it over, and
once the run is done the teammate tells you in the conversation, like any
other reply, and your phone is notified. A run that finds nothing says
nothing.

Bounds:

| | |
| --- | --- |
| once | 1 second to 30 days ahead |
| every | 15 seconds to 7 days |
| prompt | up to 8000 characters |
| jobs per teammate | 20 |

A schedule you add runs regardless of the teammate's **Background work**
setting. Cancel it from the same list.

## Let the teammate schedule itself

Turn on **Background work** in the teammate's pane. The teammate then has
`schedule`, `loop`, `list_schedules` and `cancel_schedule` as tools and can
say "check the build again in 20 minutes" and mean it. Turning it off pauses
the jobs it made without deleting them; turning it on wakes them.

## While Hotline is closed

Nothing fires while Hotline is not running. A tick missed while Hotline was closed
fires once on reopen, not once per missed interval. Closing the window does
not close Hotline; quitting from the tray does.
