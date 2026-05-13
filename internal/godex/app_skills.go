package godex

import (
	"context"

	godeskills "github.com/pandelisz/gode/internal/godex/skills"
)

func newSkillManager(cfg Config) *godeskills.Manager {
	return &godeskills.Manager{
		Workspace: cfg.Workspace,
		DataDir:   cfg.DataDir,
		LoadSettings: func(context.Context) (godeskills.ActivationSettings, error) {
			settings, err := LoadSettings(cfg.DataDir)
			if err != nil {
				return godeskills.ActivationSettings{}, err
			}
			return godeskills.ActivationSettings{
				ActiveSkills: settings.ActiveSkills,
				SkillSources: settings.SkillSources,
			}, nil
		},
		SaveSettings: func(_ context.Context, activation godeskills.ActivationSettings) error {
			settings, err := LoadSettings(cfg.DataDir)
			if err != nil {
				return err
			}
			settings.ActiveSkills = activation.ActiveSkills
			settings.SkillSources = activation.SkillSources
			return SaveSettings(cfg.DataDir, settings)
		},
	}
}
