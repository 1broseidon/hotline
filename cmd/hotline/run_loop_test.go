package main

import (
	"context"
	"errors"
	"testing"
	"time"
)

type loopServiceFunc func(context.Context) error

func (f loopServiceFunc) Run(ctx context.Context) error { return f(ctx) }

func TestRunWithLoopRunnerDrainsRunnerBeforeReturning(t *testing.T) {
	serveErr := errors.New("channel stopped")
	runnerCancelled := make(chan struct{})
	releaseRunner := make(chan struct{})

	runner := loopServiceFunc(func(ctx context.Context) error {
		<-ctx.Done()
		close(runnerCancelled)
		<-releaseRunner
		return nil
	})
	done := make(chan error, 1)
	go func() {
		done <- runWithLoopRunner(context.Background(), runner, func(context.Context) error {
			return serveErr
		})
	}()

	select {
	case <-runnerCancelled:
	case <-time.After(2 * time.Second):
		t.Fatal("channel exit did not cancel loop runner")
	}
	select {
	case err := <-done:
		t.Fatalf("returned before loop runner drained: %v", err)
	case <-time.After(50 * time.Millisecond):
	}

	close(releaseRunner)
	select {
	case err := <-done:
		if !errors.Is(err, serveErr) {
			t.Fatalf("error = %v, want %v", err, serveErr)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("did not return after loop runner drained")
	}
}

func TestRunWithLoopRunnerFailureCancelsChannelServices(t *testing.T) {
	runnerErr := errors.New("runner failed")
	serveStarted := make(chan struct{})
	serveCancelled := make(chan struct{})

	runner := loopServiceFunc(func(context.Context) error {
		<-serveStarted
		return runnerErr
	})
	err := runWithLoopRunner(context.Background(), runner, func(ctx context.Context) error {
		close(serveStarted)
		<-ctx.Done()
		close(serveCancelled)
		return nil
	})
	if !errors.Is(err, runnerErr) {
		t.Fatalf("error = %v, want wrapped %v", err, runnerErr)
	}
	select {
	case <-serveCancelled:
	case <-time.After(2 * time.Second):
		t.Fatal("loop runner failure did not cancel channel services")
	}
}
