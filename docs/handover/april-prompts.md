# Primordium prompt log, 2026-04-11 22:30 to 2026-04-17 (lost main-loop and balance work)

Extracted from `~/.claude/history.jsonl`. It contains only the user's prompts. Claude's replies and the code they produced were never pushed and are lost; the original directory was deleted on 2026-06-10. Slash commands and one-word replies are omitted. Use this as a symptom log for what the running app showed.


## 2026-04-11

- **22:32** what's next
- **22:32** branch 8 is done, check docs/plan.md and move to branch 9 : feat/main-loop
- **22:33** branch 8 is done, check docs/plan.md and move to branch 9 : feat/main-loop               DO NOT INVOKE ANY SKILL UNLESS EXPLICITLY ASKED TO.
- **22:46** what's next
- **22:48** I ran the sim, there were initial cell clusters, but they died in milliseconds
- **23:02** one more thing, before the fix, I noticed that even with running the sim multiple times, no cell moved
- **23:07** cells still don't move, they stick around for more time (around a second) before dying, one cluster also seems to reproduce all at once before vanishing
- **23:21** now the cells stay alive for much longer, around 6 seconds, but eventually all die, also most of them barely move, around 1% moves for a few pixels slowly before vanishing

## 2026-04-12

- **03:03** out of all initial colonies, only one left sustainable cells, they're moving really slowly too
- **03:07** [Image #1] how it looks after a few minutes of simulation, this is before your fix
- **03:11** [Image #2] after fix
- **03:12** but it seems to still converge to only photosynthesizers
- **03:15** don't do that, revert that change, I have a better idea
- **03:16** ❯ also some cells seem to just blindly start migrating in flocks to their bottom right side until they die.
- **03:25** [Image #3] the diagonal lines still appear
- **03:39** what if we add initial decay randomly scattered around the map, enough to make initial cells with scavenger trait able to survive, and remove the photosynthesis bias
- **03:39** what if we add initial decay randomly scattered around the map, enough to make initial cells with scavenger trait able to survive, and remove the photosynthesis bias, don't invoke skill
- **03:42** also vents
- **03:46** decay matter, pheromones, toxins, and vents aren't rendered at all
- **03:50** still no decay seen, I don't think it's the saturation that's the problem
- **03:56** revert the color boost that you did earlier
- **03:58** let's add a fast forward option
- **03:58** let's add a fast forward option, do not invoke skills
- **04:01** this just drops the fps and makes it feel slower
- **04:05** decay matter fades waaaay too quickly, certainly not enough to sustain scavengers, especially since right now cells tend to clump together in arc formations [Image #1]
- **04:18** how to change initial cell seeding
- **04:27** I don't see any hunters scavengers emerging
- **04:34** even after all of these modifications, the world still favours photosynthesizers, the top of the map gets a blanket of different colonies, some pure, some completely mixed colors, with some persistent colonies just under them that just don't seem to do anything, and small vent colonies
- **04:45** after all of these modifications, the world still favours photosynthesizers, the top of the map gets a blanket of different colonies, some pure, some completely mixed colors,
    with some persistent colonies just under them that just don't seem to do anything, and small vent colonies
- **04:48** I also noticed that no fast cells emerge, I want a vivid simulation with lots of cells and movement

## 2026-04-13

- **00:21** ecological
- **00:24** both new mechanics and fine tuned configs
- **00:27** let's go with A for now
- **00:32** implement
- **00:38** now all cells die after a while except some vent dwellers
- **00:44** this just delays their eventual death, however I noticed that there are no scavengers initially, no cell seems to be reducing decay amount in any tile it moves on
- **00:51** there isn't even any scavengers emerging in the first place

## 2026-04-16

- **21:05** same thing, only vent feeders and one mixed colony of photosynthesizers survive.
- **21:24** good progress, cells live much longer now, with scavenger colonies visible, but after some minutes, everything dies out with the exception of a mixed photosynthesizer colony and a small vent colony
- **21:32** it's too chaotic now, too much going on, many cells all over the place with some pure colonies, can we reduce the multiplier maybe ?
- **21:36** [Image #1] what causes the mixed colonies ?
- **21:38** how to reduce it ? change the gene to rgba ?
- **21:44** this is good for debugging but too restrictive. let's keep it as a debugging option but for regular use a hybrid of this method and the previous one.
- **21:50** [Image #2] debug mode shows that after a few ticks, the world is almost fully scavengers with some photosynthesizers scattered, and ALL predators vanish almost instantly (I noticed they weren't mobile)
- **21:58** predators still vanish, they're also completely stagnant since spawning in, no movement
- **22:05** they still never eat / never move

## 2026-04-17

- **21:11** continue where we left off
- **21:14** now the predators survive for a while and take over but then they all die once food is scarce and scavengers take over again
- **21:16** start with hunger gating, ignore the preference to do hands-on for now, do it all yourself
- **21:18** they still die out and scavengers thrive afterwards, maybe give them higher starvation resistance by default
- **21:22** oh I found the issue, there are too many predators at the start so they nearly wipe out the rest of the cells, maybe make the spawner biased against predators
- **21:24** they still explode in population and then die out
- **21:27** make a prompt for claude 4.7 xhigh to balance the simulation so that all cells live in harmony, no cell variety overtakes all.
- **21:28** before that, revert the latest change
- **21:29** go ahead
- **21:32** [Pasted text #1 +232 lines]
  
  [pasted #1: content not retained]
- **21:33** [Pasted text #1 +232 lines]
  
  [pasted #1: content not retained]
- **21:42** apply the fix
- **21:48** predators still die out, I noticed they're not very mobile
- **23:10** predators still die out even with prey abundant, the conservation cap is the next suspect. The cheap diagnostic: low-energy prey (just-reproduced cells with energy ≈ 40) yield only 40 × 0.7 × 0.2 = 5.6
     energy/kill. Fix = add a biomass floor: absorbed.min(defender_energy_before + 30.0) so kills always yield at least the structural-matter minimum.
- **23:12** yes for both
- **23:19** predators seem to feed a bit in their immediate spawn location, empty it, roam aimlessly even with prey a few pixels away, and die
- **23:23** fix A, B and C
- **23:40** photosynthesizers don't survive too
