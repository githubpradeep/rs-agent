import pygame
import random
import sys

# Constants
WIDTH, HEIGHT = 640, 480
BLOCK_SIZE = 20
SNAKE_SPEED = 10

# Colors
BLACK = (0, 0, 0)
WHITE = (255, 255, 255)
GREEN = (0, 255, 0)
RED = (255, 0, 0)

pygame.init()
screen = pygame.display.set_mode((WIDTH, HEIGHT))
pygame.display.set_caption('Snake Game')
clock = pygame.time.Clock()
font = pygame.font.SysFont(None, 30)

def draw_rect(color, pos):
    rect = pygame.Rect(pos[0], pos[1], BLOCK_SIZE, BLOCK_SIZE)
    pygame.draw.rect(screen, color, rect)

def spawn_food(snake_positions):
    while True:
        food = (random.randint(0, (WIDTH - BLOCK_SIZE) // BLOCK_SIZE) * BLOCK_SIZE,
                random.randint(0, (HEIGHT - BLOCK_SIZE) // BLOCK_SIZE) * BLOCK_SIZE)
        if food not in snake_positions:
            return food

def main():
    # Initial snake position (center of screen)
    snake = [(WIDTH // 2, HEIGHT // 2)]
    direction = (BLOCK_SIZE, 0)  # moving right
    food = spawn_food(snake)
    score = 0

    running = True
    while running:
        for event in pygame.event.get():
            if event.type == pygame.QUIT:
                pygame.quit()
                sys.exit()
            elif event.type == pygame.KEYDOWN:
                if event.key == pygame.K_UP and direction != (0, BLOCK_SIZE):
                    direction = (0, -BLOCK_SIZE)
                elif event.key == pygame.K_DOWN and direction != (0, -BLOCK_SIZE):
                    direction = (0, BLOCK_SIZE)
                elif event.key == pygame.K_LEFT and direction != (BLOCK_SIZE, 0):
                    direction = (-BLOCK_SIZE, 0)
                elif event.key == pygame.K_RIGHT and direction != (-BLOCK_SIZE, 0):
                    direction = (BLOCK_SIZE, 0)

        # Move snake
        head = snake[0]
        new_head = (head[0] + direction[0], head[1] + direction[1])

        # Check wall collision
        if (new_head[0] < 0 or new_head[0] >= WIDTH or
            new_head[1] < 0 or new_head[1] >= HEIGHT):
            running = False
            continue

        # Check self collision
        if new_head in snake:
            running = False
            continue

        snake.insert(0, new_head)

        # Check food collision
        if new_head == food:
            score += 1
            food = spawn_food(snake)
        else:
            snake.pop()

        # Draw
        screen.fill(BLACK)
        draw_rect(GREEN, food)
        for segment in snake:
            draw_rect(WHITE, segment)

        # Render score
        score_text = font.render(f'Score: {score}', True, WHITE)
        screen.blit(score_text, (10, 10))

        pygame.display.flip()
        clock.tick(SNAKE_SPEED)

    # Game over screen
    game_over_font = pygame.font.SysFont(None, 50)
    game_over_text = game_over_font.render('Game Over', True, RED)
    screen.blit(game_over_text, (WIDTH // 2 - game_over_text.get_width() // 2, HEIGHT // 2))
    pygame.display.flip()
    pygame.time.wait(2000)

if __name__ == '__main__':
    main()