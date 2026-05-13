package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/pandelisz/gode/internal/godex"
	godeskills "github.com/pandelisz/gode/internal/godex/skills"
)

func runSkills(ctx context.Context, args []string) error {
	if len(args) == 0 {
		return fmt.Errorf("usage: gode skills list|enable|disable|add|recommended|install-recommended")
	}
	switch args[0] {
	case "list":
		return runSkillsList(args[1:])
	case "enable":
		return runSkillsSetEnabled(args[1:], true)
	case "disable":
		return runSkillsSetEnabled(args[1:], false)
	case "add":
		return runSkillsAdd(ctx, args[1:])
	case "recommended":
		return runSkillsRecommended(args[1:])
	case "install-recommended":
		return runSkillsInstallRecommended(ctx, args[1:])
	default:
		return fmt.Errorf("unknown skills command %q", args[0])
	}
}

func runSkillsList(args []string) error {
	flags := newFlagSet("gode skills list")
	cfg := godex.DefaultConfig()
	jsonOutput := false
	bindConfigFlags(flags, &cfg)
	flags.BoolVar(&jsonOutput, "json", false, "print JSON")
	if err := flags.Parse(args); err != nil {
		return err
	}
	manager, cfg, err := skillsManagerFromFlags(cfg, flags)
	if err != nil {
		return err
	}
	items, err := manager.List(context.Background())
	if err != nil {
		return err
	}
	if jsonOutput {
		return json.NewEncoder(os.Stdout).Encode(items)
	}
	for _, skill := range items {
		if skill.Name == "" {
			fmt.Printf("diagnostic\t%s\t%s\n", skill.Path, skill.Diagnostic)
			continue
		}
		fmt.Printf("%s\t%s\t%s\t%s\t%s\n", skillScope(skill.Path, cfg), skill.State, skill.Name, oneLine(skill.Description), skill.Path)
	}
	return nil
}

func runSkillsSetEnabled(args []string, enabled bool) error {
	flags := newFlagSet("gode skills enable")
	cfg := godex.DefaultConfig()
	name := ""
	if len(args) > 0 && !strings.HasPrefix(args[0], "-") {
		name = args[0]
		args = args[1:]
	}
	bindConfigFlags(flags, &cfg)
	if err := flags.Parse(args); err != nil {
		return err
	}
	if name == "" && flags.NArg() == 1 {
		name = flags.Arg(0)
	}
	if name == "" || flags.NArg() > 1 {
		return fmt.Errorf("usage: gode skills enable|disable <name>")
	}
	manager, _, err := skillsManagerFromFlags(cfg, flags)
	if err != nil {
		return err
	}
	if err := manager.SetEnabled(context.Background(), name, enabled); err != nil {
		return err
	}
	fmt.Printf("%s\t%s\n", stateVerb(enabled), name)
	return nil
}

func runSkillsAdd(ctx context.Context, args []string) error {
	flags := newFlagSet("gode skills add")
	cfg := godex.DefaultConfig()
	var global bool
	var project bool
	var dryRun bool
	source := ""
	if len(args) > 0 && !strings.HasPrefix(args[0], "-") {
		source = args[0]
		args = args[1:]
	}
	bindConfigFlags(flags, &cfg)
	flags.BoolVar(&global, "global", false, "install to data-dir skills")
	flags.BoolVar(&project, "project", false, "install to workspace .agents/skills")
	flags.BoolVar(&dryRun, "dry-run", false, "print installer command without executing it")
	if err := flags.Parse(args); err != nil {
		return err
	}
	if source == "" && flags.NArg() == 1 {
		source = flags.Arg(0)
	}
	if source == "" || flags.NArg() > 1 {
		return fmt.Errorf("usage: gode skills add <source> [--global|--project] [--skill name] [--yes]")
	}
	manager, cfg, err := skillsManagerFromFlags(cfg, flags)
	if err != nil {
		return err
	}
	scope := installScope(global, project)
	if dryRun {
		fmt.Println(shellJoin(godeskills.NPXInstallCommand(source, scope, cfg.Workspace, cfg.DataDir)))
		return nil
	}
	result, err := manager.Install(ctx, godeskills.InstallRequest{Source: source, Scope: scope})
	if err != nil {
		return err
	}
	for _, installed := range result.Installed {
		fmt.Printf("installed\t%s\t%s\n", installed.Name, installed.Path)
	}
	if len(result.Installed) == 0 {
		fmt.Printf("installed\t%s\n", result.Source)
	}
	return nil
}

func runSkillsRecommended(args []string) error {
	flags := newFlagSet("gode skills recommended")
	cfg := godex.DefaultConfig()
	jsonOutput := false
	bindConfigFlags(flags, &cfg)
	flags.BoolVar(&jsonOutput, "json", false, "print JSON")
	if err := flags.Parse(args); err != nil {
		return err
	}
	manager, _, err := skillsManagerFromFlags(cfg, flags)
	if err != nil {
		return err
	}
	items, err := manager.Recommended(context.Background())
	if err != nil {
		return err
	}
	if jsonOutput {
		return json.NewEncoder(os.Stdout).Encode(items)
	}
	for _, item := range items {
		fmt.Printf("%s\t%s\t%s\n", item.State, item.Name, item.Source)
	}
	return nil
}

func runSkillsInstallRecommended(ctx context.Context, args []string) error {
	flags := newFlagSet("gode skills install-recommended")
	cfg := godex.DefaultConfig()
	all := false
	dryRun := false
	var names stringListFlag
	bindConfigFlags(flags, &cfg)
	flags.BoolVar(&all, "all", false, "install all missing recommended skills")
	flags.BoolVar(&dryRun, "dry-run", false, "print installer commands without executing them")
	flags.Var(&names, "skill", "recommended skill name to install; may be repeated")
	if err := flags.Parse(args); err != nil {
		return err
	}
	if !all && len(names) == 0 {
		return fmt.Errorf("usage: gode skills install-recommended --all|--skill <name> [--dry-run]")
	}
	manager, cfg, err := skillsManagerFromFlags(cfg, flags)
	if err != nil {
		return err
	}
	selected := recommendedSelection(names, all)
	if dryRun {
		for _, item := range godeskills.RecommendedDefaultSkills {
			if len(selected) > 0 {
				if _, ok := selected[item.Name]; !ok {
					continue
				}
			}
			fmt.Println(shellJoin(godeskills.NPXInstallCommand(item.Source, godeskills.InstallScopeGlobal, cfg.Workspace, cfg.DataDir)))
		}
		return nil
	}
	results, err := manager.InstallRecommended(ctx, names)
	if err != nil {
		return err
	}
	for _, result := range results {
		fmt.Printf("installed\t%s\n", result.Source)
	}
	return nil
}

func skillsManagerFromFlags(cfg godex.Config, flags *flag.FlagSet) (*godeskills.Manager, godex.Config, error) {
	loaded, err := loadConfigFromFlags(cfg, flags)
	if err != nil {
		return nil, cfg, err
	}
	cfg = loaded.Config
	manager := &godeskills.Manager{
		Workspace: cfg.Workspace,
		DataDir:   cfg.DataDir,
		LoadSettings: func(context.Context) (godeskills.ActivationSettings, error) {
			settings, err := godex.LoadSettings(cfg.DataDir)
			if err != nil {
				return godeskills.ActivationSettings{}, err
			}
			return godeskills.ActivationSettings{ActiveSkills: settings.ActiveSkills, SkillSources: settings.SkillSources}, nil
		},
		SaveSettings: func(_ context.Context, activation godeskills.ActivationSettings) error {
			settings, err := godex.LoadSettings(cfg.DataDir)
			if err != nil {
				return err
			}
			settings.ActiveSkills = activation.ActiveSkills
			settings.SkillSources = activation.SkillSources
			return godex.SaveSettings(cfg.DataDir, settings)
		},
	}
	return manager, cfg, nil
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

func installScope(global bool, project bool) godeskills.InstallScope {
	if project {
		return godeskills.InstallScopeProject
	}
	return godeskills.InstallScopeGlobal
}

func stateVerb(enabled bool) string {
	if enabled {
		return "enabled"
	}
	return "disabled"
}

func recommendedSelection(names []string, all bool) map[string]struct{} {
	if all {
		return nil
	}
	selected := map[string]struct{}{}
	for _, name := range names {
		name = strings.TrimSpace(name)
		if name != "" {
			selected[name] = struct{}{}
		}
	}
	return selected
}

func shellJoin(command []string) string {
	parts := make([]string, 0, len(command))
	for _, part := range command {
		parts = append(parts, shellQuote(part))
	}
	return strings.Join(parts, " ")
}

func shellQuote(value string) string {
	if value == "" {
		return "''"
	}
	if strings.IndexFunc(value, func(r rune) bool {
		return !(r == '-' || r == '_' || r == '/' || r == '.' || r == '@' || r == ':' || r == '=' || r == ',' || r == '+' || r >= '0' && r <= '9' || r >= 'A' && r <= 'Z' || r >= 'a' && r <= 'z')
	}) == -1 {
		return value
	}
	return "'" + strings.ReplaceAll(value, "'", "'\\''") + "'"
}
