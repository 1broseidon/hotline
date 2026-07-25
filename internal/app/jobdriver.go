package app

import (
	"errors"
	"strings"
	"time"

	"github.com/1broseidon/hotline/internal/mcpchan"
)

// JobDriver adapts the app channel's job registry to the automatic jobspool
// dispatcher. Its method set matches jobspool.JobSink structurally, so the
// dispatcher drives the SAME cards the manual `job` MCP tool drives — one card
// lifecycle, one registry — without either package importing the other.
type JobDriver struct{ t *Tools }

// JobDriver exposes the app provider's job registry to the jobspool dispatcher.
func (p *Provider) JobDriver() *JobDriver { return &JobDriver{t: p.tools} }

func (d *JobDriver) resolveChat(chatID string) string {
	if strings.TrimSpace(chatID) == "" {
		return unifiedChatID
	}
	return chatID
}

// StartCard creates a running card. It errors (rather than opening a phantom
// card) when no device can receive it, so the dispatcher retries once one links.
func (d *JobDriver) StartCard(title, detail, chatID string, progress *float64) (jobID, msgID, elementID string, err error) {
	chatID = d.resolveChat(chatID)
	title = strings.TrimSpace(title)
	if title == "" {
		title = "Subagent work"
	}
	if !d.t.srv.chatAllowed(chatID) {
		return "", "", "", errors.New("no active device")
	}
	if progress != nil && (*progress < 0 || *progress > 1) {
		progress = nil
	}
	jobID, msgID, elementID = d.t.startCard(title, strings.TrimSpace(detail), chatID, progress, time.Now().Unix())
	return jobID, msgID, elementID, nil
}

// UpdateCard is a silent edit of a running card.
func (d *JobDriver) UpdateCard(jobID, detail, chatID string, progress *float64) error {
	msg, isErr := d.t.jobUpdate(mcpchan.JobInput{
		Action: "update", JobID: jobID, Detail: detail, Progress: progress,
		ChatID: d.resolveChat(chatID),
	})
	if isErr {
		return errors.New(msg)
	}
	return nil
}

// DoneCard terminalizes a card (ok|err|cancelled) with an optional buzz line.
//
// Every closure through this path is AUTOMATIC — a completion hook, the lease
// reaper, or the restart sweep decided it, not the agent — so it is recorded as
// correctable: a later explicit `job done` from the agent that owns the work may
// still overwrite it. See jobs.go finishedJob.
func (d *JobDriver) DoneCard(jobID, state, detail, notify, chatID string) error {
	msg, isErr := d.t.finishJob(mcpchan.JobInput{
		Action: "done", JobID: jobID, State: state, Detail: detail, Notify: notify,
		ChatID: d.resolveChat(chatID),
	}, true)
	if isErr {
		return errors.New(msg)
	}
	return nil
}

// RehydrateCard re-registers a card that survived a restart so the following
// Update/Done resolves against the surviving message instead of erroring.
func (d *JobDriver) RehydrateCard(jobID, elementID, msgID, chatID, title, detail string, startedAt int64, progress *float64) {
	d.t.srv.jobs.rehydrate(&jobRecord{
		jobID: jobID, elementID: elementID, messageID: msgID, chatID: d.resolveChat(chatID),
		title: title, detail: detail, progress: progress, startedAt: startedAt,
	})
	d.t.srv.elIndex.record(msgID, []Element{jobElement(elementID, title, "running", detail, startedAt, progress)})
}
