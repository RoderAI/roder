package godex

import (
	"context"

	godeskills "github.com/pandelisz/gode/internal/godex/skills"
)

func newSkillManager(cfg Config) *godeskills.Manager {
	return &godeskills.Manager{
		Workspace: cfg.Workspace,
		DataDir:   cfg.DataDir,
		HomeDir:   cfg.HomeDir,
		LoadSettings: func(context.Context) (godeskills.ActivationSettings, error) {
			settings, err := LoadSettings(cfg.DataDir)
			if err != nil {
				return godeskills.ActivationSettings{}, err
			}
			return godeskills.ActivationSettings{
				Skills:       settings.Skills,
				SkillSources: settings.SkillSources,
			}, nil
		},
		SaveSettings: func(_ context.Context, activation godeskills.ActivationSettings) error {
			settings, err := LoadSettings(cfg.DataDir)
			if err != nil {
				return err
			}
			settings.Skills = activation.Skills
			settings.SkillSources = activation.SkillSources
			return SaveSettings(cfg.DataDir, settings)
		},
	}
}

func loadSkillsConfig(dataDir string) func(context.Context) (godeskills.Config, error) {
	return func(context.Context) (godeskills.Config, error) {
		settings, err := LoadSettings(dataDir)
		if err != nil {
			return godeskills.Config{}, err
		}
		return settings.Skills, nil
	}
}
