package main

import (
	"context"
	"flag"
	"fmt"
	"path/filepath"
	"strings"

	"github.com/pandelisz/gode/internal/godex"
	godeskills "github.com/pandelisz/gode/internal/godex/skills"
)

func runSkills(ctx context.Context, args []string) error {
	if len(args) == 0 {
		return fmt.Errorf("usage: gode skills list|add")
	}
	switch args[0] {
	case "list":
		return runSkillsList(args[1:])
	case "add":
		return runSkillsAdd(ctx, args[1:])
	default:
		return fmt.Errorf("unknown skills command %q", args[0])
	}
}

func runSkillsList(args []string) error {
	flags := newFlagSet("gode skills list")
	cfg := godex.DefaultConfig()
	bindConfigFlags(flags, &cfg)
	if err := flags.Parse(args); err != nil {
		return err
	}
	loaded, err := loadConfigFromFlags(cfg, flags)
	if err != nil {
		return err
	}
	cfg = loaded.Config
	catalog := godeskills.Discover(godeskills.DiscoverOptions{Workspace: cfg.Workspace, DataDir: cfg.DataDir})
	for _, skill := range catalog.Skills {
		fmt.Printf("%s\t%s\t%s\t%s\n", skillScope(skill.Path, cfg), skill.Name, oneLine(skill.Description), skill.Path)
	}
	for _, diag := range catalog.Diagnostics {
		fmt.Printf("diagnostic\t%s\t%s\n", diag.Path, diag.Message)
	}
	return nil
}

func runSkillsAdd(ctx context.Context, args []string) error {
	flags := newFlagSet("gode skills add")
	cfg := godex.DefaultConfig()
	var global bool
	var project bool
	var yes bool
	var names stringListFlag
	source := ""
	if len(args) > 0 && !strings.HasPrefix(args[0], "-") {
		source = args[0]
		args = args[1:]
	}
	bindConfigFlags(flags, &cfg)
	flags.BoolVar(&global, "global", false, "install to data-dir skills")
	flags.BoolVar(&project, "project", false, "install to workspace .agents/skills")
	flags.BoolVar(&yes, "yes", false, "confirm installing all skills from a multi-skill source")
	flags.Var(&names, "skill", "skill name to install; may be repeated")
	if err := flags.Parse(args); err != nil {
		return err
	}
	if source == "" && flags.NArg() == 1 {
		source = flags.Arg(0)
	}
	if source == "" || flags.NArg() > 1 {
		return fmt.Errorf("usage: gode skills add <source> [--global|--project] [--skill name] [--yes]")
	}
	loaded, err := loadConfigFromFlags(cfg, flags)
	if err != nil {
		return err
	}
	cfg = loaded.Config
	result, err := godeskills.Install(ctx, godeskills.InstallOptions{
		Source:     source,
		Workspace:  cfg.Workspace,
		DataDir:    cfg.DataDir,
		Global:     global,
		Project:    project,
		SkillNames: names,
		Yes:        yes,
	})
	if err != nil {
		return err
	}
	for _, installed := range result.Installed {
		fmt.Printf("installed\t%s\t%s\n", installed.Name, installed.Path)
	}
	return nil
}

func skillScope(path string, cfg godex.Config) string {
	clean := filepath.Clean(path)
	workspace := filepath.Clean(cfg.Workspace)
	dataDir := filepath.Clean(cfg.DataDir)
	switch {
	case strings.HasPrefix(clean, filepath.Join(workspace, ".agents", "skills")):
		return "project"
	case strings.Contains(clean, string(filepath.Separator)+".agents"+string(filepath.Separator)+"skills"+string(filepath.Separator)):
		return "project"
	case strings.HasPrefix(clean, filepath.Join(workspace, ".gode", "skills")):
		return "project"
	case strings.HasPrefix(clean, filepath.Join(dataDir, "skills")):
		return "global"
	default:
		return "external"
	}
}

type stringListFlag []string

func (f *stringListFlag) String() string {
	return strings.Join(*f, ",")
}

func (f *stringListFlag) Set(value string) error {
	value = strings.TrimSpace(value)
	if value != "" {
		*f = append(*f, value)
	}
	return nil
}

var _ flag.Value = (*stringListFlag)(nil)
